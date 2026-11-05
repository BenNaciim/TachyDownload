//! HLS (`hls.js`): a stream is a playlist of small pieces. They are fetched over several connections, put back in
//! order (each connection takes the next piece as soon as it is free, even when an earlier one is slow; what
//! arrives out of turn waits in memory, up to a cap) and appended to one file, `.ts` or `.mp4` for fragmented MP4.
//! With ffmpeg installed the raw pieces become a regular `.mp4` without re-encoding. Plain AES-128 is decrypted;
//! DRM and live streams are refused with a clear message.

use super::*;
use crate::errors::MSG_INVALID_URL;
use crate::state::HlsProgress;
use aes::Aes128;
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

const HLS_RETRIES: u32 = 4;
const DRM: &str = "Vidéo protégée (DRM) : le téléchargement n’est pas possible";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub url: String,
    pub iv: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub url: String,
    pub key: Option<Key>,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub url: String,
    pub bandwidth: u64,
    pub resolution: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub segments: Vec<Item>,
    pub map: Option<Item>,
    pub ended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Playlist {
    /// Variants, the best bandwidth first.
    Master(Vec<Variant>),
    Media(Media),
}

/// What the details view draws: which segments are in flight and which were received ahead.
#[derive(Debug, Default)]
pub struct Live {
    pub running: BTreeSet<usize>,
    pub ready: BTreeSet<usize>,
}

fn parse_attrs(line: &str) -> HashMap<String, String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"([A-Z0-9-]+)=("([^"]*)"|[^,]*)"#).unwrap());
    let rest = line.split_once(':').map_or("", |(_, r)| r);
    re.captures_iter(rest)
        .map(|c| {
            (
                c[1].to_string(),
                c.get(3)
                    .or_else(|| c.get(2))
                    .map_or("", |m| m.as_str())
                    .to_string(),
            )
        })
        .collect()
}

/// Parses an m3u8 playlist: either a master (variants) or a media playlist (segments).
pub fn parse_playlist(text: &str, base_url: &str) -> std::result::Result<Playlist, TaskError> {
    let lines: Vec<&str> = text
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.first() != Some(&"#EXTM3U") {
        return Err(TaskError::fatal("Ce n’est pas une playlist HLS valide"));
    }
    let base = url::Url::parse(base_url).map_err(|_| TaskError::fatal(MSG_INVALID_URL))?;
    let abs = |u: &str| -> std::result::Result<String, TaskError> {
        base.join(u)
            .map(|u| u.to_string())
            .map_err(|_| TaskError::fatal(MSG_INVALID_URL))
    };

    if lines.iter().any(|l| l.starts_with("#EXT-X-STREAM-INF")) {
        let mut variants = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if !l.starts_with("#EXT-X-STREAM-INF") {
                continue;
            }
            let a = parse_attrs(l);
            if let Some(uri) = lines[i + 1..].iter().find(|l| !l.starts_with('#')) {
                variants.push(Variant {
                    url: abs(uri)?,
                    bandwidth: a.get("BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0),
                    resolution: a.get("RESOLUTION").cloned().unwrap_or_default(),
                });
            }
        }
        variants.sort_by_key(|v| std::cmp::Reverse(v.bandwidth));
        return Ok(Playlist::Master(variants));
    }

    let mut pl = Media {
        segments: Vec::new(),
        map: None,
        ended: false,
    };
    let mut media_sequence = 0u64;
    let mut key: Option<Key> = None;
    for l in lines {
        if let Some(n) = l.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            media_sequence = n.trim().parse().unwrap_or(0);
        } else if l == "#EXT-X-ENDLIST" {
            pl.ended = true;
        } else if l.starts_with("#EXT-X-KEY:") {
            let a = parse_attrs(l);
            match (a.get("METHOD").map(String::as_str), a.get("URI")) {
                (Some("NONE"), _) => key = None,
                (Some("AES-128"), Some(uri)) if !uri.is_empty() => {
                    key = Some(Key {
                        url: abs(uri)?,
                        iv: a.get("IV").cloned().filter(|v| !v.is_empty()),
                    });
                }
                _ => return Err(TaskError::fatal(DRM)),
            }
        } else if l.starts_with("#EXT-X-MAP:") {
            let a = parse_attrs(l);
            if let Some(uri) = a.get("URI").filter(|u| !u.is_empty()) {
                pl.map = Some(Item {
                    url: abs(uri)?,
                    key: key.clone(),
                    seq: 0,
                });
            }
        } else if !l.starts_with('#') {
            pl.segments.push(Item {
                url: abs(l)?,
                key: key.clone(),
                seq: media_sequence + pl.segments.len() as u64,
            });
        }
    }
    Ok(Playlist::Media(pl))
}

/// The IV of a segment: the one the playlist gives, or its sequence number as a 128-bit big-endian value.
fn iv_for(key: &Key, seq: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    if let Some(text) = &key.iv {
        let hex = text
            .strip_prefix("0x")
            .or_else(|| text.strip_prefix("0X"))
            .unwrap_or(text);
        let padded = format!("{hex:0>32}");
        for (i, b) in iv.iter_mut().enumerate() {
            *b = padded
                .get(i * 2..i * 2 + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .unwrap_or(0);
        }
    } else {
        iv[12..].copy_from_slice(&(seq as u32).to_be_bytes());
    }
    iv
}

fn decrypt(buf: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> std::result::Result<Vec<u8>, TaskError> {
    cbc::Decryptor::<Aes128>::new(key.into(), iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(buf)
        .map_err(|_| TaskError::new("Segment illisible"))
}

type Keys = Arc<std::sync::Mutex<HashMap<String, [u8; 16]>>>;

async fn pause(ms: u64, cancel: &CancellationToken) -> Result<()> {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(ms)) => Ok(()),
        _ = cancel.cancelled() => Err(Fail::Cancelled),
    }
}

impl Task {
    /// The extra fields the window reads for a stream: one lane with the overall progress, and the segments.
    pub(super) fn hls_snapshot(
        &self,
        state: &mut TaskState,
        downloaded: u64,
        more: &mut serde_json::Map<String, Value>,
    ) {
        // One lane showing overall progress; the size is an estimate until the end.
        let total = state.size.unwrap_or(0).max(downloaded).max(1);
        state.segments = Some(vec![Segment {
            start: 0,
            end: total - 1,
            pos: downloaded,
        }]);
        let hls = state.hls.unwrap_or_default();
        more.insert(
            "hlsProgress".into(),
            if hls.total > 0 {
                json!(format!("{} / {} segments", hls.next, hls.total))
            } else {
                Value::Null
            },
        );
        more.insert("hlsTotal".into(), json!(hls.total));
        more.insert("hlsDone".into(), json!(hls.next));
        let live = lock(&self.hls_live);
        let (active, all): (usize, Vec<usize>) = match live.as_ref() {
            Some(l) => (
                l.running.len(),
                l.running.union(&l.ready).copied().collect(),
            ),
            None => (0, vec![]),
        };
        more.insert("hlsActive".into(), json!(active));
        more.insert("hlsLive".into(), json!(all));
    }

    async fn hls_text(&self, url: &str, cancel: &CancellationToken) -> Result<(String, String)> {
        let headers = self.base_headers(url);
        let res = self.ctx.net().get(url, &headers, cancel).await?;
        let status = res.status().as_u16();
        if status >= 400 {
            return Err(Fail::Error(TaskError::http(status)));
        }
        let final_url = res.url().to_string();
        let text = tokio::select! {
            _ = cancel.cancelled() => return Err(Fail::Cancelled),
            t = res.text() => t.map_err(|e| TaskError::from_reqwest(&e))?,
        };
        Ok((text, final_url))
    }

    async fn hls_key(
        &self,
        url: &str,
        keys: &Keys,
        cancel: &CancellationToken,
    ) -> Result<[u8; 16]> {
        if let Some(k) = lock(keys).get(url) {
            return Ok(*k);
        }
        let headers = self.base_headers(url);
        let res = self.ctx.net().get(url, &headers, cancel).await?;
        if res.status().as_u16() >= 400 {
            return Err(Fail::Error(TaskError::fatal(
                "Clé de déchiffrement inaccessible",
            )));
        }
        let bytes = tokio::select! {
            _ = cancel.cancelled() => return Err(Fail::Cancelled),
            b = res.bytes() => b.map_err(|e| TaskError::from_reqwest(&e))?,
        };
        let key: [u8; 16] = bytes
            .as_ref()
            .try_into()
            .map_err(|_| Fail::Error(TaskError::fatal(DRM)))?;
        lock(keys).insert(url.to_string(), key);
        Ok(key)
    }

    /// One piece, whole, decrypted, with a few retries.
    async fn hls_bytes(
        self: Arc<Self>,
        item: Item,
        keys: Keys,
        cancel: CancellationToken,
    ) -> Result<Vec<u8>> {
        let mut attempt = 0u32;
        loop {
            let one = async {
                let headers = self.base_headers(&item.url);
                let res = self.ctx.net().get(&item.url, &headers, &cancel).await?;
                let status = res.status().as_u16();
                if status >= 500 || status == 429 {
                    return Err(Fail::Error(TaskError::new(format!("HTTP {status}"))));
                }
                if status >= 400 {
                    return Err(Fail::Error(TaskError::http(status)));
                }
                let bytes = tokio::select! {
                    _ = cancel.cancelled() => return Err(Fail::Cancelled),
                    b = res.bytes() => b.map_err(|e| TaskError::from_reqwest(&e))?,
                };
                let mut buf = bytes.to_vec();
                if let Some(key) = &item.key {
                    let k = self.hls_key(&key.url, &keys, &cancel).await?;
                    buf = decrypt(&buf, &k, &iv_for(key, item.seq))?;
                }
                self.take(buf.len(), &cancel).await?;
                Ok(buf)
            };
            match one.await {
                Ok(buf) => return Ok(buf),
                Err(Fail::Cancelled) => return Err(Fail::Cancelled),
                Err(Fail::Error(e)) => {
                    attempt += 1;
                    if cancel.is_cancelled() || e.fatal || attempt > HLS_RETRIES {
                        return Err(Fail::Error(e));
                    }
                    pause((500u64 << attempt).min(6000), &cancel).await?;
                }
            }
        }
    }

    pub(super) async fn run_hls(self: &Arc<Self>, cancel: CancellationToken) -> Result<()> {
        *lock(&self.hls_live) = Some(Live::default());
        let result = self.run_hls_inner(&cancel).await;
        *lock(&self.hls_live) = None;
        result
    }

    async fn run_hls_inner(self: &Arc<Self>, cancel: &CancellationToken) -> Result<()> {
        static EXT: OnceLock<Regex> = OnceLock::new();
        static FMP4: OnceLock<Regex> = OnceLock::new();
        let (dir, url, connections) = {
            let s = lock(&self.state);
            (s.dir.clone(), s.url.clone(), s.connections)
        };
        std::fs::create_dir_all(&dir)?;

        let (text, final_url) = self.hls_text(&url, cancel).await?;
        let mut pl = parse_playlist(&text, &final_url)?;
        if let Playlist::Master(variants) = &pl {
            let Some(best) = variants.first() else {
                return Err(Fail::Error(TaskError::fatal("Playlist sans flux vidéo")));
            };
            let (text, final_url) = self.hls_text(&best.url, cancel).await?;
            pl = parse_playlist(&text, &final_url)?;
        }
        let Playlist::Media(media) = pl else {
            return Err(Fail::Error(TaskError::fatal(
                "Playlist HLS non prise en charge",
            )));
        };
        if !media.ended {
            return Err(Fail::Error(TaskError::fatal(
                "Les directs ne sont pas pris en charge",
            )));
        }
        if media.segments.is_empty() {
            return Err(Fail::Error(TaskError::fatal("Playlist vide")));
        }
        let mut items: Vec<Item> = Vec::with_capacity(media.segments.len() + 1);
        if let Some(map) = &media.map {
            items.push(map.clone());
        }
        items.extend(media.segments.iter().cloned());
        let fmp4 = media.map.is_some()
            || FMP4
                .get_or_init(|| Regex::new(r"(?i)\.(m4s|mp4|cmfv|cmfa)(\?|$)").unwrap())
                .is_match(&items[items.len() - 1].url);

        {
            let mut s = lock(&self.state);
            let ext = EXT
                .get_or_init(|| Regex::new(r"(?i)\.(m3u8?|ts|mp4|mkv|webm)$").unwrap())
                .replace(&s.filename, "")
                .into_owned();
            s.filename = format!("{ext}{}", if fmp4 { ".mp4" } else { ".ts" });
            let part_exists = self.part_path_of(&s).exists();
            let total = items.len() as u64;
            let mut hls = s.hls.unwrap_or(HlsProgress {
                next: 0,
                bytes: 0,
                total,
            });
            if hls.total != total || !part_exists {
                hls = HlsProgress {
                    next: 0,
                    bytes: 0,
                    total,
                };
            }
            s.hls = Some(hls);
        }
        self.clear_samples();
        let part = self.part_path();
        let progress = lock(&self.state).hls.unwrap_or_default();
        let file = Arc::new(if progress.next > 0 {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&part)?
        } else {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&part)?
        });
        file.set_len(progress.bytes)?; // drop a partially written tail from an earlier run

        let _ticker = {
            let this = self.clone();
            AbortOnDrop(tokio::spawn(async move {
                let mut iv = tokio::time::interval_at(
                    tokio::time::Instant::now() + Duration::from_millis(250),
                    Duration::from_millis(250),
                );
                loop {
                    iv.tick().await;
                    this.emit();
                }
            }))
        };

        // Every connection takes the next segment as soon as it is free, even when an earlier one is still slow:
        // segments that arrive out of turn wait in memory until it is theirs to be written, up to a cap.
        let window = (connections as usize).max(1);
        let max_ahead = self
            .ctx
            .hls_ahead_bytes
            .load(std::sync::atomic::Ordering::Relaxed);
        let keys: Keys = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let workers = cancel.child_token();
        let mut set: JoinSet<(usize, Result<Vec<u8>>)> = JoinSet::new();
        let mut running: BTreeSet<usize> = BTreeSet::new();
        let mut ready: HashMap<usize, Vec<u8>> = HashMap::new();
        let mut buffered = 0u64;
        let mut fetch_idx = progress.next as usize;
        let mut failure: Option<Fail> = None;
        let publish = |running: &BTreeSet<usize>, ready: &HashMap<usize, Vec<u8>>| {
            if let Some(l) = lock(&self.hls_live).as_mut() {
                l.running = running.clone();
                l.ready = ready.keys().copied().collect();
            }
        };
        let total = items.len();
        loop {
            // The segment the writer waits for is always fetched before any later one, so the cap cannot starve it.
            while failure.is_none()
                && running.len() < window
                && fetch_idx < items.len()
                && buffered < max_ahead
            {
                let i = fetch_idx;
                fetch_idx += 1;
                running.insert(i);
                let (this, item, keys, token) = (
                    self.clone(),
                    items[i].clone(),
                    keys.clone(),
                    workers.clone(),
                );
                set.spawn(async move { (i, this.hls_bytes(item, keys, token).await) });
            }
            publish(&running, &ready);
            let next = lock(&self.state).hls.unwrap_or_default().next as usize;
            if next >= total {
                break;
            }
            if let Some(f) = failure.take() {
                workers.cancel();
                return Err(f);
            }
            if let Some(buf) = ready.remove(&next) {
                buffered -= buf.len() as u64;
                let offset = lock(&self.state).hls.unwrap_or_default().bytes;
                let len = buf.len() as u64;
                let f = file.clone();
                tokio::task::spawn_blocking(move || write_all_at(&f, &buf, offset))
                    .await
                    .map_err(|e| TaskError::new(e.to_string()))??;
                {
                    let mut s = lock(&self.state);
                    let mut h = s.hls.unwrap_or_default();
                    h.bytes += len;
                    h.next += 1;
                    s.size = Some(((h.bytes as f64 / h.next as f64) * total as f64).round() as u64);
                    s.hls = Some(h);
                }
                lock(&self.samples).push_back((Instant::now(), len));
                continue;
            }
            let joined = tokio::select! {
                biased;
                _ = cancel.cancelled() => { workers.cancel(); return Err(Fail::Cancelled); }
                j = set.join_next() => j,
            };
            match joined {
                Some(Ok((i, Ok(buf)))) => {
                    running.remove(&i);
                    buffered += buf.len() as u64;
                    ready.insert(i, buf);
                }
                Some(Ok((i, Err(Fail::Cancelled)))) => {
                    running.remove(&i);
                    if cancel.is_cancelled() {
                        return Err(Fail::Cancelled);
                    }
                }
                Some(Ok((i, Err(e)))) => {
                    running.remove(&i);
                    failure.get_or_insert(e);
                }
                Some(Err(e)) => {
                    failure.get_or_insert(Fail::Error(TaskError::new(format!(
                        "Erreur interne : {e}"
                    ))));
                }
                None => {
                    // nothing in flight and the next segment is not here: cannot happen, but never wait forever
                    return Err(Fail::Error(TaskError::new("Flux interrompu")));
                }
            }
        }
        workers.cancel();
        let f = file.clone();
        tokio::task::spawn_blocking(move || f.sync_all())
            .await
            .map_err(|e| TaskError::new(e.to_string()))??;
        drop(file);
        if cancel.is_cancelled() {
            return Err(Fail::Cancelled);
        }

        self.hls_convert(cancel).await?;

        let part = self.part_path();
        let size = {
            let mut s = lock(&self.state);
            let name = s.filename.clone();
            let final_path = settle_finished(&self.ctx, &mut s, &part, &name)?;
            let size = std::fs::metadata(&final_path)?.len();
            s.size = Some(size);
            let mut h = s.hls.unwrap_or_default();
            h.bytes = size;
            s.hls = Some(h);
            size
        };
        let _ = size;
        self.set(Status::Completed, None);
        Ok(())
    }

    /// With ffmpeg available, the raw pieces become a regular .mp4 (stream copy, no re-encoding). A failed
    /// conversion keeps the raw file rather than lose the download.
    async fn hls_convert(&self, cancel: &CancellationToken) -> Result<()> {
        let Some(ffmpeg) = self.ctx.ffmpeg() else {
            return Ok(());
        };
        if !self.ctx.settings().convert_to_mp4 {
            return Ok(());
        }
        lock(&self.state).phase = Some("convert".into());
        self.emit();
        let part = self.part_path();
        let converted = PathBuf::from(format!("{}.mp4", part.display()));
        let mut cmd = tokio::process::Command::new(ffmpeg);
        cmd.args(["-y", "-v", "error", "-i"])
            .arg(&part)
            .args(["-c", "copy", "-f", "mp4", "-movflags", "+faststart"])
            .arg(&converted)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let outcome = tokio::select! {
            _ = cancel.cancelled() => None,
            r = cmd.status() => Some(r),
        };
        let result = match outcome {
            None => {
                let _ = std::fs::remove_file(&converted);
                lock(&self.state).phase = None;
                return Err(Fail::Cancelled);
            }
            Some(Ok(st)) if st.success() => {
                let raw = self.part_path();
                {
                    let mut s = lock(&self.state);
                    static EXT: OnceLock<Regex> = OnceLock::new();
                    let stem = EXT
                        .get_or_init(|| Regex::new(r"(?i)\.(ts|mp4)$").unwrap())
                        .replace(&s.filename, "")
                        .into_owned();
                    s.filename = format!("{stem}.mp4");
                }
                let new_part = self.part_path(); // follows the new name
                std::fs::rename(&converted, &new_part)?;
                if raw != new_part {
                    let _ = std::fs::remove_file(raw);
                }
                Ok(())
            }
            _ => {
                let _ = std::fs::remove_file(&converted);
                Ok(())
            }
        };
        lock(&self.state).phase = None;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_master_lists_variants_best_first_with_urls_resolved() {
        let pl = parse_playlist(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1920x1080\nhigh/index.m3u8\n",
            "http://h/a/master.m3u8",
        )
        .unwrap();
        let Playlist::Master(v) = pl else {
            panic!("a master")
        };
        assert_eq!(v[0].url, "http://h/a/high/index.m3u8");
        assert_eq!(v[0].bandwidth, 3_000_000);
        assert_eq!(v[0].resolution, "1920x1080");
        assert_eq!(v[1].url, "http://h/a/low/index.m3u8");
    }

    #[test]
    fn a_media_playlist_numbers_its_segments_and_remembers_keys_and_the_init_map() {
        let pl = parse_playlist(
            "#EXTM3U\r\n#EXT-X-MEDIA-SEQUENCE:7\r\n#EXT-X-MAP:URI=\"init.mp4\"\r\n#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\",IV=0x0A\r\n#EXTINF:4,\r\na.m4s\r\n#EXT-X-KEY:METHOD=NONE\r\n#EXTINF:4,\r\nb.m4s\r\n#EXT-X-ENDLIST\r\n",
            "http://h/v/p.m3u8",
        )
        .unwrap();
        let Playlist::Media(m) = pl else {
            panic!("a media playlist")
        };
        assert!(m.ended);
        assert_eq!(m.map.unwrap().url, "http://h/v/init.mp4");
        assert_eq!(m.segments[0].seq, 7);
        assert_eq!(m.segments[1].seq, 8);
        let key = m.segments[0].key.clone().unwrap();
        assert_eq!(key.url, "http://h/v/k.bin");
        assert_eq!(key.iv.as_deref(), Some("0x0A"));
        assert!(m.segments[1].key.is_none());
    }

    #[test]
    fn drm_and_foreign_files_are_refused_with_a_clear_message() {
        let drm = parse_playlist(
            "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\n#EXTINF:4,\nseg0.ts\n",
            "http://h/p.m3u8",
        )
        .unwrap_err();
        assert!(drm.fatal && drm.message.contains("DRM"));
        let not_hls = parse_playlist("<html></html>", "http://h/p.m3u8").unwrap_err();
        assert_eq!(not_hls.message, "Ce n’est pas une playlist HLS valide");
    }

    #[test]
    fn the_iv_is_explicit_padded_or_the_sequence_number() {
        let k = |iv: Option<&str>| Key {
            url: "x".into(),
            iv: iv.map(str::to_string),
        };
        let mut want = [0u8; 16];
        want[15] = 0x0a;
        assert_eq!(iv_for(&k(Some("0x0A")), 99), want);
        let mut seq = [0u8; 16];
        seq[15] = 7;
        assert_eq!(iv_for(&k(None), 7), seq);
        let mut big = [0u8; 16];
        big[12..].copy_from_slice(&1u32.to_be_bytes());
        assert_eq!(iv_for(&k(None), (1u64 << 32) + 1), big);
    }
}
