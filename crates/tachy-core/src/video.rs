//! Video pages (`ytdlp.js`, the part that does not run a download): which addresses yt-dlp handles, what the user
//! may choose for one video, the pieces of the yt-dlp command line, what yt-dlp prints and how to read it, the
//! French messages for its failures, and the two questions asked before a download (look at a video, list a
//! playlist).

use crate::ctx::YtdlpLaunch;
use crate::state::{Frag, ReplayHeaders, TaskState, Ytd};
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

fn re(pattern: &str) -> Regex {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .expect("a valid pattern")
}

// ---- which addresses ----------------------------------------------------------------------------------------

/// True for the address of a video page that yt-dlp handles and the browser cannot give as a plain file.
pub fn is_video_page(url: &str) -> bool {
    use std::sync::OnceLock;
    static PAGES: OnceLock<Vec<Regex>> = OnceLock::new();
    PAGES
        .get_or_init(|| {
            [
                r"^https?://(www\.|m\.|music\.)?youtube\.com/(watch|shorts/|live/)",
                r"^https?://youtu\.be/[\w-]+",
                r"^https?://(www\.)?vimeo\.com/\d+",
                r"^https?://(www\.)?dailymotion\.com/video/",
                r"^https?://(www\.)?twitch\.tv/videos/\d+",
            ]
            .iter()
            .map(|p| re(p))
            .collect()
        })
        .iter()
        .any(|r| r.is_match(url))
}

/// True for the address of a YouTube playlist page (not a video that happens to carry `list=`).
pub fn is_playlist_url(url: &str) -> bool {
    use std::sync::OnceLock;
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"^https?://(www\.|m\.|music\.)?youtube\.com/playlist\?(.*&)?list=[\w-]+"))
        .is_match(url)
}

// ---- what the user may choose -------------------------------------------------------------------------------

/// The options of one video, read from the stored object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct YtOpts {
    pub audio_only: bool,
    pub height: Option<u32>,
    pub lang: Option<String>,
}

impl YtOpts {
    pub fn from_value(v: Option<&Value>) -> YtOpts {
        let Some(o) = v.and_then(Value::as_object) else {
            return YtOpts::default();
        };
        YtOpts {
            audio_only: o.get("audioOnly") == Some(&Value::Bool(true)),
            height: o
                .get("height")
                .and_then(Value::as_u64)
                .map(|h| h as u32)
                .filter(|h| *h > 0),
            lang: o.get("lang").and_then(Value::as_str).map(str::to_string),
        }
    }
}

const HEIGHTS: [u64; 9] = [144, 240, 360, 480, 720, 1080, 1440, 2160, 4320];

/// What the user may choose for one video; anything else is dropped. None when nothing is left.
pub fn clean_yt_opts(o: &Value) -> Option<Value> {
    use std::sync::OnceLock;
    static LANG: OnceLock<Regex> = OnceLock::new();
    let o = o.as_object()?;
    let mut out = Map::new();
    if o.get("audioOnly") == Some(&Value::Bool(true)) {
        out.insert("audioOnly".into(), Value::Bool(true));
    }
    // `Number(o.height)`: a number or a numeric string
    let height = match o.get("height") {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    if let Some(h) = height.filter(|h| HEIGHTS.iter().any(|k| *k as f64 == *h)) {
        out.insert("height".into(), Value::from(h as u64));
    }
    let lang = LANG.get_or_init(|| Regex::new(r"^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})?$").unwrap());
    if let Some(l) = o
        .get("lang")
        .and_then(Value::as_str)
        .filter(|l| lang.is_match(l))
    {
        out.insert("lang".into(), Value::String(l.to_string()));
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

// ---- the command line ---------------------------------------------------------------------------------------

/// The yt-dlp format arguments for the chosen options (pure, so it can be tested without yt-dlp).
pub fn format_args(opts: &YtOpts, max_height: u32, has_ffmpeg: bool) -> Vec<String> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let lang = opts.lang.as_deref().map(|l| format!("lang:{l}"));
    if opts.audio_only {
        // Converting to mp3 needs ffmpeg; without it the original audio (usually .m4a) is kept.
        let mut a = if has_ffmpeg {
            s(&[
                "-f",
                "ba/b",
                "-x",
                "--audio-format",
                "mp3",
                "--audio-quality",
                "0",
            ])
        } else {
            s(&["-f", "ba[ext=m4a]/ba/b"])
        };
        if let Some(l) = lang {
            a.extend(["-S".to_string(), l]);
        }
        return a;
    }
    let h = opts.height.unwrap_or(max_height);
    if has_ffmpeg {
        // Prefer H.264 + AAC: plays everywhere on Windows, unlike AV1/Opus in an mp4.
        let sort = format!(
            "{}res:{h},vcodec:h264,acodec:m4a",
            lang.map(|l| format!("{l},")).unwrap_or_default()
        );
        let mut a = s(&["--merge-output-format", "mp4", "-f", "bv*+ba/b", "-S"]);
        a.push(sort);
        return a;
    }
    vec!["-f".into(), format!("b[height<={h}]/b")] // one file with sound: no merge tool needed
}

/// The request context yt-dlp should replay for a page or a manifest it does not know: what the browser sent
/// with it (Referer, Origin, User-Agent). Never for the big video sites, which want none of it. Only clean values
/// pass.
pub fn header_args(headers: Option<&ReplayHeaders>, url: &str) -> Vec<String> {
    use std::sync::OnceLock;
    static REFERER: OnceLock<Regex> = OnceLock::new();
    static ORIGIN: OnceLock<Regex> = OnceLock::new();
    let Some(h) = headers else {
        return vec![];
    };
    if is_video_page(url) {
        return vec![];
    }
    let referer = REFERER.get_or_init(|| re(r"^https?://\S+$"));
    let origin = ORIGIN.get_or_init(|| re(r"^https?://[^\s/]+$"));
    let mut out = vec![];
    if referer.is_match(&h.referer) {
        out.extend(["--referer".to_string(), h.referer.clone()]);
    }
    if origin.is_match(&h.origin) {
        out.extend(["--add-headers".to_string(), format!("Origin:{}", h.origin)]);
    }
    let ua = &h.user_agent;
    if !ua.is_empty() && !ua.starts_with('-') && ua.chars().all(|c| (' '..='~').contains(&c)) {
        out.extend(["--user-agent".to_string(), ua.clone()]);
    }
    out
}

/// The command line of a video download. The lower of the shared speed limit and the video's own goes to
/// yt-dlp, which reads it at start: a change applies on resume.
pub fn ytdlp_args(
    s: &TaskState,
    cfg: &crate::settings::Settings,
    limit_kbps: u64,
    ffmpeg: Option<&Path>,
    deno: Option<&Path>,
) -> Vec<String> {
    let max_height = if cfg.video_max_height > 0 {
        cfg.video_max_height
    } else {
        1080
    };
    let opts = YtOpts::from_value(s.yt_opts.as_ref());
    let own = |v: &str| v.to_string();
    let mut args: Vec<String> = [
        // --print implies --quiet, which would silence the progress lines: --progress asks for them again.
        "--newline",
        "--progress",
        "--no-colors",
        "--encoding",
        "utf-8",
        "--no-playlist",
        "--no-mtime",
        "--no-simulate",
        "--progress-template",
        "download:DMP %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s %(progress.fragment_index)s %(progress.fragment_count)s",
        "--print",
        "before_dl:DMT %(title)s",
        "--print",
        "after_move:DMF %(filepath)s",
        "-P",
    ]
    .iter()
    .map(|a| own(a))
    .collect();
    args.push(s.dir.clone());
    args.push("-o".into());
    // A name chosen for the file ("%" doubled: it is a yt-dlp template) or the video's own title and id.
    args.push(match &s.out_stem {
        Some(stem) if !stem.is_empty() => format!("{}.%(ext)s", stem.replace('%', "%%")),
        _ => "%(title).150B [%(id)s].%(ext)s".into(),
    });
    args.push("--concurrent-fragments".into());
    args.push(s.connections.max(1).to_string());
    // A video downloaded again lands on the file it replaces: without this, yt-dlp calls it "already downloaded".
    if s.replaces.is_some() {
        args.push("--force-overwrites".into());
    }
    if let Some(ff) = ffmpeg {
        args.push("--ffmpeg-location".into());
        args.push(
            ff.parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
    }
    args.extend(format_args(&opts, max_height, ffmpeg.is_some()));
    // Without a JS engine YouTube hides the H.264 formats and may hide all of them.
    if let Some(d) = deno {
        args.push("--js-runtimes".into());
        args.push(format!("deno:{}", d.display()));
    }
    if !cfg.proxy.is_empty() {
        args.push("--proxy".into());
        args.push(cfg.proxy.clone());
    }
    if !cfg.yt_cookies.is_empty() {
        // a signed-in browser passes YouTube's bot check
        args.push("--cookies-from-browser".into());
        args.push(cfg.yt_cookies.clone());
    }
    args.extend(header_args(s.headers.as_ref(), &s.url));
    let caps = [limit_kbps, s.speed_limit_kbps.unwrap_or(0)];
    if let Some(cap) = caps.into_iter().filter(|v| *v > 0).min() {
        args.push("--limit-rate".into());
        args.push(format!("{cap}K"));
    }
    args.push(s.url.clone());
    args
}

// ---- what yt-dlp prints -------------------------------------------------------------------------------------

/// Folds one `DMP done total estimate fragIndex fragCount` line of yt-dlp into the task state (`ytd`, `frag`,
/// `size`) and returns the bytes gained since the previous line, for the speed. `first` marks the first line of a
/// run: it counts what an earlier run left on disk, which is not speed.
pub fn apply_progress(s: &mut TaskState, line: &str, first: bool) -> u64 {
    let mut parts = line.split(' ');
    parts.next(); // "DMP"
    // `Number("NA") || 0`: whatever is not a number reads as zero
    let mut num = || {
        parts
            .next()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|n| n.is_finite())
            .unwrap_or(0.0)
    };
    let (done, total, est, frag_index, frag_count) = (num(), num(), num(), num(), num());
    let y = s.ytd.get_or_insert_with(Ytd::default);
    let cur = done;
    // Next stream (audio after video) restarts near zero: keep what the finished one really weighed. Parallel
    // fragments make the figure wobble by a few KB, so only a real fall counts as a new stream.
    if cur < y.cur * 0.5 {
        y.base += y.cur;
    }
    let gained = if cur > y.cur && !first {
        (cur - y.cur) as u64
    } else {
        0
    };
    y.cur = cur;
    // Fragment tally for the details pane: the index wobbles with parallel fragments, so it only grows within a
    // stream.
    if frag_count > 0.0 {
        let fc = frag_count as u64;
        let before = s.frag.filter(|f| f.total == fc).map_or(0, |f| f.done);
        s.frag = Some(Frag {
            done: fc.min((frag_index as u64).max(before)),
            total: fc,
        });
    }
    // Fragmented streams (most of YouTube) give no byte total: extrapolate from the fragments done. With parallel
    // fragments the index lags far behind the bytes at the start, so wait for a few fragments before trusting the
    // ratio (otherwise the total explodes to terabytes).
    let by_fragments = if frag_index >= 5.0f64.max(frag_count * 0.03) && frag_count >= frag_index {
        (cur * frag_count / frag_index).round()
    } else {
        0.0
    };
    y.total = if total != 0.0 {
        total
    } else if est != 0.0 {
        est
    } else {
        by_fragments
    };
    let downloaded = y.base + y.cur;
    // unknown until a trustworthy estimate exists
    s.size = Some(if y.total != 0.0 {
        (y.base + y.total).max(downloaded).round() as u64
    } else {
        0
    });
    gained
}

/// Turns yt-dlp's English messages into something actionable; None when the cause is unknown.
pub fn explain(msg: &str, url: &str) -> Option<String> {
    let m = msg;
    // without an address, the usual suspect
    let youtube = url.is_empty() || re(r"youtube\.com|youtu\.be").is_match(url);
    let head = |n: usize| m.chars().take(n).collect::<String>();
    if re(r"not a bot|sign in to confirm").is_match(m) {
        return Some("YouTube demande de prouver que vous n’êtes pas un robot. Dans Paramètres → Vidéos, choisissez « Cookies du navigateur » (un navigateur où vous êtes connecté à YouTube), puis réessayez.".into());
    }
    if re(r"confirm your age|inappropriate for some users").is_match(m) {
        return Some("Vidéo réservée aux adultes : YouTube exige une connexion.".into());
    }
    if re(r"private video|members-only|join this channel").is_match(m) {
        return Some("Vidéo privée ou réservée aux abonnés.".into());
    }
    if re(r"unsupported url").is_match(m) {
        return Some("yt-dlp ne trouve aucune vidéo sur cette page. Ouvrez-la dans le navigateur avec l’extension, lancez la lecture, puis cliquez sur « Télécharger cette vidéo » : l’extension repère le flux pendant qu’il joue.".into());
    }
    if re(r"video unavailable|this video is unavailable|has been removed|no longer available|does not exist").is_match(m) {
        return Some("Vidéo indisponible ou supprimée.".into());
    }
    if re(r"not available in your country|blocked it in your country").is_match(m) {
        return Some("Vidéo non disponible dans votre pays.".into());
    }
    if re(r"live event will begin|is live|premieres in").is_match(m) {
        return Some("Direct ou première pas encore terminés : téléchargement impossible.".into());
    }
    if re(r"requested format is not available|no video formats|javascript runtime|n challenge")
        .is_match(m)
    {
        return Some(format!(
            "{} donné aucun format téléchargeable pour cette vidéo ({}).",
            if youtube {
                "YouTube n’a"
            } else {
                "Le site n’a"
            },
            head(160)
        ));
    }
    if re(r"http error 403|forbidden").is_match(m) {
        return Some(if youtube {
            "YouTube a refusé le téléchargement (403). Mettez yt-dlp à jour (Paramètres → Vidéos) ou choisissez les cookies d’un navigateur connecté.".into()
        } else {
            "Le site a refusé le téléchargement (403) : l’adresse a sans doute expiré, ou il exige la page d’origine. Relancez la lecture dans le navigateur avec l’extension, qui enverra une adresse neuve.".into()
        });
    }
    if re(r"ffmpeg|postprocessing|merge").is_match(m) {
        return Some(format!(
            "La fusion de l’audio et de la vidéo a échoué : vérifiez que ffmpeg est installé (Paramètres → Vidéos). ({})",
            head(120)
        ));
    }
    if re(r"no space left|disk full|not enough space").is_match(m) {
        return Some("Disque plein : libérez de la place puis réessayez.".into());
    }
    if re(r"unable to download|getaddrinfo|timed out|network is unreachable|connection").is_match(m)
    {
        return Some(format!("Problème de connexion : {}", head(160)));
    }
    None
}

/// The text of an `ERROR:` line of yt-dlp without its prefix (and without the `[extractor]` tag), or none for any
/// other line.
pub fn error_text(line: &str) -> Option<String> {
    let r = re(r"^ERROR:\s*(\[[^\]]+\]\s*)?");
    r.is_match(line).then(|| r.replace(line, "").into_owned())
}

/// A warning line without its `WARNING:` prefix.
pub fn warning_text(line: &str) -> String {
    re(r"^WARNING:\s*").replace(line, "").into_owned()
}

/// The id of a video, from the file name yt-dlp announced (`... [id].ext`) or from the address.
pub fn video_id(final_file: Option<&str>, url: &str) -> Option<String> {
    let from_name = re(r"\[([\w-]{11})\]\.[A-Za-z0-9]+$")
        .captures(final_file.unwrap_or(""))
        .map(|c| c[1].to_string());
    from_name.or_else(|| {
        re(r"[?&]v=([\w-]{11})|youtu\.be/([\w-]{11})")
            .captures(url)
            .and_then(|c| {
                c.get(1)
                    .or_else(|| c.get(2))
                    .map(|m| m.as_str().to_string())
            })
    })
}

/// True when yt-dlp's complaint is a lost connection or a server error: worth another try later.
pub fn is_retryable(detail: &str) -> bool {
    re(r"getaddrinfo|timed out|network is unreachable|connection (reset|aborted|refused)|http error 5\d\d")
        .is_match(detail)
}

/// What the diagnostic file holds: what was run, everything yt-dlp said, how it ended.
pub struct Diag<'a> {
    pub url: &'a str,
    pub args: &'a [String],
    pub code: i32,
    pub final_file: Option<&'a str>,
    pub file_found: bool,
    pub error: &'a str,
    pub lines: &'a [String],
    pub last_progress: &'a str,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// Text of the diagnostic file. Pure, so it can be tested.
pub fn format_diag(d: &Diag) -> String {
    let mut out = vec![
        format!(
            "Tachy Download – diagnostic yt-dlp, {}",
            d.at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        ),
        format!("Adresse : {}", d.url),
        format!("Arguments : {}", d.args.join(" ")),
        format!("Code de sortie : {}", d.code),
        format!(
            "Fichier annoncé : {} — {}",
            d.final_file.unwrap_or("(aucun)"),
            if d.file_found {
                "présent"
            } else {
                "introuvable"
            }
        ),
        format!(
            "Erreur affichée : {}",
            if d.error.is_empty() {
                "(aucune)"
            } else {
                d.error
            }
        ),
        if d.last_progress.is_empty() {
            String::new()
        } else {
            format!("Dernière progression : {}", d.last_progress)
        },
        "--- sortie de yt-dlp ---".into(),
    ];
    out.extend(d.lines.iter().cloned());
    out.push(String::new());
    out.join("\n")
}

/// The newest finished file in `dir` written since `since_ms` whose name holds "[id].", or none.
pub fn find_by_video_id(dir: &Path, id: &str, since_ms: u64) -> Option<PathBuf> {
    use std::sync::OnceLock;
    static UNFINISHED: OnceLock<Regex> = OnceLock::new();
    if id.is_empty() {
        return None;
    }
    let unfinished = UNFINISHED.get_or_init(|| re(r"\.(part|ytdl|temp)$|\.part-"));
    let mut best: Option<(PathBuf, u64)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.contains(&format!("[{id}].")) || unfinished.is_match(&name) {
            continue;
        }
        let t = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        if t < since_ms {
            continue;
        }
        if best.as_ref().is_none_or(|(_, bt)| t > *bt) {
            best = Some((e.path(), t));
        }
    }
    best.map(|(p, _)| p)
}

// ---- the plain file behind a video --------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectFile {
    pub url: String,
    pub ext: String,
    pub title: String,
    pub user_agent: String,
}

/// The plain file behind a video when yt-dlp's chosen format is one http(s) file (no merge, no fragments): the
/// app can then fetch it itself over several connections. None for anything else.
pub fn direct_file(info: &Value) -> Option<DirectFile> {
    let truthy = |k: &str| !matches!(info.get(k), None | Some(Value::Null | Value::Bool(false)));
    if truthy("requested_formats") || truthy("fragments") {
        return None;
    }
    let url = info.get("url")?.as_str()?;
    if !re(r"^https?://").is_match(url) {
        return None;
    }
    if !re(r"^https?$").is_match(info.get("protocol").and_then(Value::as_str).unwrap_or("")) {
        return None;
    }
    let ext = info.get("ext").and_then(Value::as_str).unwrap_or("");
    let ext = if re(r"^[a-z0-9]{2,5}$").is_match(ext) {
        ext.to_ascii_lowercase()
    } else {
        "mp4".into()
    };
    if ext == "m3u8" || ext == "mpd" {
        return None;
    }
    let title = info
        .get("title")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .unwrap_or("video");
    Some(DirectFile {
        url: url.to_string(),
        ext,
        title: title.to_string(),
        user_agent: info
            .pointer("/http_headers/User-Agent")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

// ---- looking at a video ------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Lang {
    pub code: String,
    pub label: String,
}

/// What the window shows to let the user choose: title, duration, available heights and audio languages.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Probe {
    pub title: String,
    pub duration: f64,
    pub heights: Vec<u32>,
    pub langs: Vec<Lang>,
}

pub fn summarize_probe(info: &Value) -> Probe {
    let formats = info
        .get("formats")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let codec = |f: &Value, k: &str| {
        f.get(k)
            .and_then(Value::as_str)
            .is_some_and(|c| !c.is_empty() && c != "none")
    };
    let mut heights: Vec<u32> = formats
        .iter()
        .filter(|f| codec(f, "vcodec"))
        .filter_map(|f| f.get("height").and_then(Value::as_f64))
        .filter(|h| *h > 0.0)
        .map(|h| h as u32)
        .collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();
    let mut langs: Vec<(String, String)> = vec![];
    for f in &formats {
        if !codec(f, "acodec") {
            continue;
        }
        let Some(code) = f
            .get("language")
            .and_then(Value::as_str)
            .filter(|l| !l.is_empty())
        else {
            continue;
        };
        let note = f.get("format_note").and_then(Value::as_str).unwrap_or("");
        let note = note.split(',').next().unwrap_or("");
        let label = re(r"\s*\(default\)").replace(note, "").trim().to_string();
        match langs.iter_mut().find(|(c, _)| c == code) {
            None => langs.push((code.to_string(), label)),
            Some((_, l)) if l.is_empty() && !label.is_empty() => *l = label,
            Some(_) => {}
        }
    }
    Probe {
        title: info
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        duration: info
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|d| d.is_finite())
            .unwrap_or(0.0),
        heights,
        langs: langs
            .into_iter()
            .map(|(code, label)| Lang {
                label: if label.is_empty() {
                    code.clone()
                } else {
                    label
                },
                code,
            })
            .collect(),
    }
}

// ---- running yt-dlp for a quick answer ----------------------------------------------------------------------

/// yt-dlp as it must be started.
#[derive(Clone)]
pub struct YtdlpBin {
    pub exe: PathBuf,
    pub launch: YtdlpLaunch,
}

impl YtdlpBin {
    /// A command ready to run: no console window, UTF-8 output (without these, yt-dlp prints non-ASCII titles
    /// through the Windows code page and mangles them), nothing read from the keyboard.
    pub fn command(&self, args: &[String]) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.exe);
        cmd.args(&self.launch.prefix)
            .args(args)
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .envs(self.launch.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        cmd
    }

    /// Runs yt-dlp to the end and returns (succeeded, stdout, stderr); an error when it cannot be started or takes
    /// longer than `timeout`.
    pub async fn run(
        &self,
        args: &[String],
        timeout: Duration,
    ) -> Result<(bool, String, String), String> {
        let mut cmd = self.command(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let out = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| "yt-dlp ne répond pas".to_string())?
            .map_err(|e| e.to_string())?;
        Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    }
}

/// What a probe passes to yt-dlp besides the address.
#[derive(Default)]
pub struct ProbeOpts<'a> {
    pub deno: Option<&'a Path>,
    pub cookies: &'a str,
    pub proxy: &'a str,
    pub headers: Option<&'a ReplayHeaders>,
}

/// The arguments shared by the questions asked of yt-dlp: no playlist, UTF-8, JS engine, proxy, cookies, headers.
pub fn question_args(url: &str, o: &ProbeOpts) -> Vec<String> {
    let mut args: Vec<String> = [
        "-J",
        "--no-playlist",
        "--no-warnings",
        "--encoding",
        "utf-8",
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    if let Some(d) = o.deno {
        args.push("--js-runtimes".into());
        args.push(format!("deno:{}", d.display()));
    }
    if !o.cookies.is_empty() {
        args.push("--cookies-from-browser".into());
        args.push(o.cookies.to_string());
    }
    if !o.proxy.is_empty() {
        args.push("--proxy".into());
        args.push(o.proxy.to_string());
    }
    args.extend(header_args(o.headers, url));
    args.push(url.to_string());
    args
}

/// Looks at a video without downloading it: title, available heights and audio languages.
pub async fn probe_video(yt: &YtdlpBin, url: &str, o: &ProbeOpts<'_>) -> Result<Probe, String> {
    let (ok, stdout, stderr) = yt
        .run(&question_args(url, o), Duration::from_secs(60))
        .await?;
    if !ok {
        let line = stderr
            .lines()
            .find(|l| re("error").is_match(l))
            .unwrap_or(&stderr)
            .to_string();
        let plain: String = re(r"^ERROR:\s*(\[[^\]]+\]\s*)?")
            .replace(&line, "")
            .chars()
            .take(200)
            .collect();
        return Err(explain(&line, url).unwrap_or(plain));
    }
    serde_json::from_str::<Value>(&stdout)
        .map(|v| summarize_probe(&v))
        .map_err(|_| "Réponse de yt-dlp illisible".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistItem {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistInfo {
    pub title: String,
    pub items: Vec<PlaylistItem>,
}

/// Lists a playlist without downloading it: its videos plus the playlist's own title.
pub async fn list_playlist(
    yt: &YtdlpBin,
    url: &str,
    limit: u32,
    proxy: &str,
) -> Result<PlaylistInfo, String> {
    let mut args: Vec<String> = ["--flat-playlist", "--no-warnings"]
        .iter()
        .map(|a| a.to_string())
        .collect();
    if !proxy.is_empty() {
        args.push("--proxy".into());
        args.push(proxy.to_string());
    }
    args.extend([
        "--playlist-end".to_string(),
        limit.to_string(),
        "--print".to_string(),
        "%(playlist_title)s\t%(url)s\t%(title)s".to_string(),
        url.to_string(),
    ]);
    let (ok, stdout, stderr) = yt.run(&args, Duration::from_secs(90)).await?;
    if !ok {
        let first = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .map(|l| {
                re(r"^ERROR:\s*(\[[^\]]+\]\s*)?")
                    .replace(l, "")
                    .into_owned()
            })
            .filter(|l| !l.is_empty());
        return Err(first.unwrap_or_else(|| "Playlist illisible".into()));
    }
    let rows: Vec<Vec<&str>> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').collect())
        .collect();
    let items = rows
        .iter()
        .filter(|r| r.get(1).is_some_and(|u| re(r"^https?://").is_match(u)))
        .map(|r| PlaylistItem {
            url: r[1].to_string(),
            title: r.get(2).unwrap_or(&"").to_string(),
        })
        .collect();
    let title = rows
        .first()
        .and_then(|r| r.first())
        .filter(|t| !t.is_empty() && **t != "NA")
        .map_or("Playlist".to_string(), |t| t.to_string());
    Ok(PlaylistInfo { title, items })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn video_page_detection() {
        assert!(is_video_page("https://www.youtube.com/watch?v=abc"));
        assert!(is_video_page("https://youtu.be/abc"));
        assert!(!is_video_page("https://example.com/watch?v=abc"));
        assert!(!is_video_page("https://www.youtube.com/"));
        assert!(is_video_page("https://vimeo.com/12345"));
        assert!(is_video_page("https://www.twitch.tv/videos/99"));
    }

    #[test]
    fn playlist_detection_only_real_playlist_pages_not_a_video_that_carries_list() {
        assert!(is_playlist_url(
            "https://www.youtube.com/playlist?list=PLabc_123-x"
        ));
        assert!(!is_playlist_url(
            "https://www.youtube.com/watch?v=abc&list=PLabc"
        ));
    }

    #[test]
    fn clean_yt_opts_keeps_only_known_options() {
        assert_eq!(
            clean_yt_opts(
                &json!({ "audioOnly": true, "height": 720, "lang": "fr", "evil": "--exec" })
            ),
            Some(json!({ "audioOnly": true, "height": 720, "lang": "fr" }))
        );
        assert_eq!(
            clean_yt_opts(&json!({ "height": 999, "lang": "--exec calc" })),
            None
        );
        assert_eq!(clean_yt_opts(&Value::Null), None);
        assert_eq!(
            clean_yt_opts(&json!({ "height": 0, "lang": "en-US" })),
            Some(json!({ "lang": "en-US" }))
        );
        assert_eq!(
            clean_yt_opts(&json!({ "height": "480" })),
            Some(json!({ "height": 480 })),
            "Number(\"480\") is 480"
        );
    }

    fn opts(h: Option<u32>, lang: Option<&str>, audio: bool) -> YtOpts {
        YtOpts {
            audio_only: audio,
            height: h,
            lang: lang.map(str::to_string),
        }
    }

    #[test]
    fn format_args_height_and_audio_language_drive_the_sort() {
        let a = format_args(&opts(Some(720), Some("fr"), false), 1080, true);
        assert_eq!(
            a[a.len() - 2..],
            ["-S", "lang:fr,res:720,vcodec:h264,acodec:m4a"]
        );
        let b = format_args(&opts(None, None, false), 1080, true);
        assert_eq!(b[b.len() - 2..], ["-S", "res:1080,vcodec:h264,acodec:m4a"]);
    }

    #[test]
    fn format_args_without_ffmpeg_a_single_file_with_sound_is_chosen() {
        assert_eq!(
            format_args(&opts(Some(480), None, false), 1080, false),
            ["-f", "b[height<=480]/b"]
        );
    }

    #[test]
    fn format_args_audio_only_becomes_mp3_with_ffmpeg_and_stays_m4a_without() {
        let with = format_args(&opts(None, None, true), 1080, true);
        assert!(with.contains(&"-x".to_string()) && with.contains(&"mp3".to_string()));
        let without = format_args(&opts(None, None, true), 1080, false);
        assert!(!without.contains(&"-x".to_string()));
        assert!(without[1].starts_with("ba"));
        let fr = format_args(&opts(None, Some("fr"), true), 1080, false);
        assert_eq!(fr[fr.len() - 2..], ["-S", "lang:fr"]);
    }

    #[test]
    fn summarize_probe_lists_heights_high_to_low_and_audio_languages_once() {
        let p = summarize_probe(&json!({
            "title": "T",
            "duration": 90,
            "formats": [
                { "vcodec": "avc1", "acodec": "none", "height": 720 },
                { "vcodec": "avc1", "acodec": "none", "height": 1080 },
                { "vcodec": "vp9", "acodec": "none", "height": 1080 },
                { "vcodec": "none", "acodec": "mp4a", "language": "fr", "format_note": "Français, medium" },
                { "vcodec": "none", "acodec": "mp4a", "language": "fr", "format_note": "Français, low" },
                { "vcodec": "none", "acodec": "mp4a", "language": "en", "format_note": "English (default), medium" },
                { "vcodec": "none", "acodec": "mp4a" },
            ],
        }));
        assert_eq!(p.heights, [1080, 720]);
        assert_eq!(
            p.langs,
            [
                Lang {
                    code: "fr".into(),
                    label: "Français".into()
                },
                Lang {
                    code: "en".into(),
                    label: "English".into()
                }
            ]
        );
        assert_eq!(p.duration, 90.0);
        assert_eq!(p.title, "T");
    }

    fn state_with(url: &str) -> TaskState {
        TaskState {
            url: url.into(),
            dir: "/d".into(),
            connections: 4,
            ..TaskState::default()
        }
    }

    fn settings(json: Value) -> crate::settings::Settings {
        crate::settings::Settings::from_patch(&json)
    }

    #[test]
    fn the_chosen_options_reach_the_yt_dlp_command_line() {
        let mut s = state_with("https://youtu.be/abc");
        s.yt_opts = Some(json!({ "height": 480, "lang": "fr" }));
        let args = ytdlp_args(
            &s,
            &settings(json!({ "videoMaxHeight": 1080 })),
            0,
            Some(Path::new("/t/ffmpeg.exe")),
            None,
        );
        assert!(args.contains(&"lang:fr,res:480,vcodec:h264,acodec:m4a".to_string()));
        assert_eq!(args.last().unwrap(), "https://youtu.be/abc");
    }

    const PLAYER: &str = "https://player.example/embed/7";
    const PLAYER_ORIGIN: &str = "https://player.example";

    fn headers(referer: &str, origin: &str, ua: &str) -> ReplayHeaders {
        ReplayHeaders {
            referer: referer.into(),
            origin: origin.into(),
            cookie: String::new(),
            user_agent: ua.into(),
        }
    }

    #[test]
    fn yt_dlp_gets_the_request_context_of_a_page_it_does_not_know_not_of_youtube() {
        let h = headers(PLAYER, PLAYER_ORIGIN, "Agent/1.0");
        let unknown = header_args(Some(&h), "https://cdn.example/v/manifest.mpd");
        assert_eq!(unknown[..2], ["--referer", PLAYER]);
        let joined = unknown.join(" ");
        assert!(joined.contains(&format!("--add-headers Origin:{PLAYER_ORIGIN}")));
        assert!(joined.contains("--user-agent Agent/1.0"));
        assert!(header_args(Some(&h), "https://www.youtube.com/watch?v=abcdefghijk").is_empty());
        assert!(header_args(None, "https://cdn.example/v.mpd").is_empty());
        assert!(
            header_args(
                Some(&headers("not a url", "x\ny", "")),
                "https://cdn.example/v.mpd"
            )
            .is_empty(),
            "nothing that is not a clean value is passed on"
        );
        assert!(
            header_args(Some(&headers("", "", "-evil")), "https://cdn.example/v.mpd").is_empty(),
            "a user agent cannot pass as an option"
        );
    }

    #[test]
    fn the_command_line_carries_those_headers_before_the_address() {
        let mut s = state_with("https://cdn.example/v/manifest.mpd");
        s.headers = Some(headers(PLAYER, PLAYER_ORIGIN, ""));
        let args = ytdlp_args(&s, &settings(json!({})), 0, None, None);
        let at = args.iter().position(|a| a == "--referer").unwrap();
        assert_eq!(args[at + 1], PLAYER);
        assert_eq!(args.last().unwrap(), "https://cdn.example/v/manifest.mpd");
    }

    #[test]
    fn a_name_chosen_for_the_file_doubles_its_percent_signs_and_a_replaced_file_is_overwritten() {
        let mut s = state_with("https://cdn.example/v.mpd");
        s.out_stem = Some("100% vrai".into());
        s.replaces = Some("/d/old.mp4".into());
        let args = ytdlp_args(&s, &settings(json!({})), 0, None, None);
        let at = args.iter().position(|a| a == "-o").unwrap();
        assert_eq!(args[at + 1], "100%% vrai.%(ext)s");
        assert!(args.contains(&"--force-overwrites".to_string()));
    }

    #[test]
    fn the_lower_of_the_shared_and_the_own_speed_limit_goes_to_yt_dlp() {
        let limit = |shared: u64, own: Option<u64>| {
            let mut s = state_with("https://youtu.be/abc");
            s.speed_limit_kbps = own;
            let a = ytdlp_args(&s, &settings(json!({})), shared, None, None);
            a.iter()
                .position(|x| x == "--limit-rate")
                .map(|i| a[i + 1].clone())
        };
        assert_eq!(limit(500, Some(200)).as_deref(), Some("200K"));
        assert_eq!(limit(100, Some(400)).as_deref(), Some("100K"));
        assert_eq!(limit(0, Some(0)), None);
        assert_eq!(limit(0, Some(300)).as_deref(), Some("300K"));
    }

    // Lines in the shape yt-dlp really printed for a long YouTube video: no byte total, an estimate that wanders,
    // and a fragment index that wobbles with 16 fragments in flight.
    fn feed(s: &mut TaskState, lines: &[&str], first: bool) -> Vec<u64> {
        lines
            .iter()
            .enumerate()
            .map(|(i, l)| apply_progress(s, &format!("DMP {l}"), first && i == 0))
            .collect()
    }

    #[test]
    fn a_long_fragmented_video_keeps_a_sane_size_when_the_byte_count_wobbles() {
        let mut s = TaskState::default();
        feed(
            &mut s,
            &[
                "271244144 NA 3815647496.33 215 3008",
                "271911792 NA 3683838307.17 221 3008",
                "1715471624 NA 3648800885.23 1415 3008",
                "1714791056 NA 3653381169.64 1413 3008", // a few KB lower: not a new stream
                "1716008200 NA 3645288152.58 1415 3008",
            ],
            true,
        );
        assert_eq!(s.ytd.unwrap().base, 0.0);
        let size = s.size.unwrap() as f64;
        assert!(
            (size - 3645288152.58).abs() < 1.0,
            "the last estimate from yt-dlp, got {size}"
        );
        assert_eq!(
            s.frag,
            Some(Frag {
                done: 1415,
                total: 3008
            }),
            "fragment count never goes back"
        );
    }

    #[test]
    fn at_the_start_the_fragment_index_lags_the_total_stays_unknown_rather_than_exploding() {
        let mut s = TaskState::default();
        feed(
            &mut s,
            &[
                "130048 NA NA 0 120",
                "307168 NA NA 1 120",
                "1034144 NA NA 2 120",
            ],
            true,
        );
        assert_eq!(s.size, Some(0), "unknown until a few fragments are done");
        feed(&mut s, &["6000000 NA NA 6 120"], false);
        assert_eq!(s.size, Some(120_000_000));
    }

    #[test]
    fn the_byte_total_or_the_yt_dlp_estimate_wins_over_our_own_extrapolation() {
        let mut s = TaskState::default();
        feed(&mut s, &["1024 62914560 NA NA NA"], true);
        assert_eq!(s.size, Some(62914560));
        let mut t = TaskState::default();
        feed(&mut t, &["1000000 NA 36000000.0 0 120"], true);
        assert_eq!(t.size, Some(36000000));
    }

    #[test]
    fn an_audio_stream_starting_after_the_video_adds_what_the_video_really_weighed() {
        let mut s = TaskState::default();
        feed(&mut s, &["500000000 500000000 NA NA NA"], true);
        feed(&mut s, &["1000 80000000 NA NA NA"], false); // second stream restarts near zero
        let y = s.ytd.unwrap();
        assert_eq!(y.base, 500000000.0);
        assert_eq!(s.size, Some(580000000));
        assert_eq!(y.base + y.cur, 500001000.0);
    }

    #[test]
    fn the_first_line_of_a_resumed_run_is_not_counted_as_speed_the_next_ones_are() {
        let mut s = TaskState::default();
        let g = feed(
            &mut s,
            &[
                "271244144 NA 3815647496 215 3008",
                "271911792 NA 3683838307 221 3008",
                "271900000 NA 3683838307 221 3008",
            ],
            true,
        );
        assert_eq!(g[0], 0);
        assert_eq!(g[1], 271911792 - 271244144);
        assert_eq!(g[2], 0, "a dip is not negative speed");
    }

    #[test]
    fn a_size_never_falls_below_what_is_already_downloaded() {
        let mut s = TaskState::default();
        feed(&mut s, &["900 NA 500 NA NA"], true);
        assert_eq!(s.size, Some(900));
    }

    #[test]
    fn format_diag_lists_what_was_run_and_how_it_ended() {
        let text = format_diag(&Diag {
            url: "u",
            args: &["-a".into(), "b".into()],
            code: 1,
            final_file: None,
            file_found: false,
            error: "boom",
            lines: &["x".into(), "y".into()],
            last_progress: "1 2",
            at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        });
        assert!(text.contains("Arguments : -a b"));
        assert!(text.contains("Fichier annoncé : (aucun) — introuvable"));
        assert!(text.contains("--- sortie de yt-dlp ---\nx\ny"));
        assert!(text.contains("1970-01-01T00:00:00.000Z"));
    }

    #[test]
    fn known_yt_dlp_messages_become_actions_unknown_ones_stay_none() {
        assert!(
            explain("HTTP Error 403: Forbidden", "")
                .unwrap()
                .contains("403")
        );
        assert!(
            explain("Postprocessing: ffprobe and ffmpeg not found", "")
                .unwrap()
                .contains("ffmpeg")
        );
        assert!(
            explain("[Errno 28] No space left on device", "")
                .unwrap()
                .contains("Disque plein")
        );
        assert!(
            explain("Sign in to confirm you're not a bot", "")
                .unwrap()
                .contains("robot")
        );
        assert_eq!(explain("something nobody has seen", ""), None);
    }

    #[test]
    fn a_page_yt_dlp_does_not_know_sends_the_user_to_the_extension_not_to_a_dead_end() {
        let msg = explain("ERROR: Unsupported URL: https://site.example/serie/1", "").unwrap();
        assert!(
            msg.to_lowercase().contains("extension"),
            "says what to do instead"
        );
        assert!(msg.contains("lecture"), "the video has to be playing");
        assert!(!msg.contains("YouTube"));
    }

    #[test]
    fn a_refused_stream_is_explained_with_its_likely_cause_for_a_site_that_is_not_youtube() {
        let msg = explain(
            "HTTP Error 403: Forbidden",
            "https://cdn.example/v/manifest.mpd",
        )
        .unwrap();
        assert!(!msg.contains("YouTube"));
        assert!(msg.contains("403"));
        assert!(
            explain(
                "HTTP Error 403: Forbidden",
                "https://www.youtube.com/watch?v=abcdefghijk"
            )
            .unwrap()
            .contains("YouTube")
        );
        assert!(
            explain("HTTP Error 403: Forbidden", "")
                .unwrap()
                .contains("YouTube"),
            "without an address, as before"
        );
    }

    #[test]
    fn only_a_single_plain_http_file_counts_as_a_direct_download() {
        assert_eq!(
            direct_file(
                &json!({ "url": "https://cdn.x/v.mp4", "protocol": "https", "ext": "mp4", "title": "T" })
            ),
            Some(DirectFile {
                url: "https://cdn.x/v.mp4".into(),
                ext: "mp4".into(),
                title: "T".into(),
                user_agent: String::new()
            })
        );
        assert_eq!(
            direct_file(
                &json!({ "url": "https://cdn.x/v.m3u8", "protocol": "m3u8_native", "ext": "mp4" })
            ),
            None
        );
        assert_eq!(
            direct_file(
                &json!({ "url": "https://cdn.x/v.mp4", "protocol": "https", "ext": "mp4", "requested_formats": [{}, {}] })
            ),
            None
        );
        assert_eq!(
            direct_file(
                &json!({ "url": "https://cdn.x/v.mp4", "protocol": "https", "ext": "mp4", "fragments": [{}] })
            ),
            None
        );
        assert_eq!(
            direct_file(&json!({ "protocol": "https", "ext": "mp4" })),
            None
        );
    }

    #[test]
    fn the_lines_yt_dlp_prints_are_read_and_the_video_id_is_found() {
        assert_eq!(
            error_text("ERROR: [youtube] abcdefghijk: Video unavailable").as_deref(),
            Some("abcdefghijk: Video unavailable")
        );
        assert_eq!(error_text("WARNING: x"), None);
        assert_eq!(warning_text("WARNING: slow"), "slow");
        assert_eq!(
            video_id(Some("/d/Episode ? [abcdefghijk].mp4"), "https://x/y").as_deref(),
            Some("abcdefghijk")
        );
        assert_eq!(
            video_id(None, "https://www.youtube.com/watch?v=abcdefghijk").as_deref(),
            Some("abcdefghijk")
        );
        assert_eq!(
            video_id(None, "https://youtu.be/abcdefghijk?t=3").as_deref(),
            Some("abcdefghijk")
        );
        assert_eq!(video_id(None, "https://site.example/x"), None);
    }

    #[test]
    fn a_retryable_complaint_is_a_lost_connection_or_a_server_error() {
        assert!(is_retryable("HTTP Error 503: Service Unavailable"));
        assert!(is_retryable("getaddrinfo failed"));
        assert!(!is_retryable("Video unavailable"));
    }
}
