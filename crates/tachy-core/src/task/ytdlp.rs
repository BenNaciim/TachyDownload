//! A download handled by the yt-dlp program (`ytdlp.js`): it finds the streams, we show its progress and keep the
//! file. A site that serves one plain file gets the app's own multi-connection download instead.

use super::*;
use crate::state::Ytd;
use crate::util::sanitize_filename;
use crate::video::{
    Diag, DirectFile, YtOpts, YtdlpBin, apply_progress, direct_file, error_text, explain,
    find_by_video_id, format_args, format_diag, header_args, is_retryable, is_video_page, video_id,
    warning_text, ytdlp_args,
};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;

const NO_TOOLS: &str = "Outils vidéo non installés : ouvrez Paramètres → Outils vidéo";

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Sends every line the stream prints, trimmed, to `tx`.
fn pump_lines<R: AsyncRead + Unpin + Send + 'static>(r: R, tx: mpsc::UnboundedSender<String>) {
    tokio::spawn(async move {
        let mut r = BufReader::new(r);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match r.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf).trim().to_string();
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

/// Ends a program and everything it started (yt-dlp starts ffmpeg).
async fn kill_tree(child: &mut tokio::process::Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let mut cmd = tokio::process::Command::new("taskkill");
        cmd.args(["/pid", &pid.to_string(), "/T", "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let _ = cmd.status().await;
    }
    let _ = child.start_kill();
}

/// What yt-dlp printed during one run.
struct Out {
    first_report: bool,
    final_file: Option<String>,
    last_err: String,
    /// Everything but progress, kept for the diagnostic file.
    lines: Vec<String>,
    last_progress: String,
    /// The last messages, for when it fails without an ERROR line.
    tail: Vec<String>,
}

impl Task {
    pub(super) async fn run_ytdlp(self: &Arc<Self>, cancel: CancellationToken) -> Result<()> {
        let Some(yt) = self.ctx.ytdlp() else {
            return Err(Fail::Error(TaskError::fatal(NO_TOOLS)));
        };
        let (dir, url, audio_only) = {
            let s = lock(&self.state);
            (
                s.dir.clone(),
                s.url.clone(),
                YtOpts::from_value(s.yt_opts.as_ref()).audio_only,
            )
        };
        std::fs::create_dir_all(&dir)?;
        // A site that serves one plain file gets the app's own multi-connection download (yt-dlp would use one).
        // The big sites hand out separate audio and video streams, so they go straight to yt-dlp.
        if !is_video_page(&url) && !audio_only {
            let direct = self.resolve_direct(&yt, &cancel).await;
            if cancel.is_cancelled() {
                return Err(Fail::Cancelled);
            }
            if let Some(d) = direct {
                match self.run_direct(&d, &cancel).await {
                    Ok(true) => return Ok(()),
                    Ok(false) => {}
                    Err(_) if cancel.is_cancelled() => return Err(Fail::Cancelled),
                    // the direct file refused us (an address bound to yt-dlp's own request, say): let yt-dlp do it
                    Err(_) => {}
                }
            }
        }
        {
            let mut s = lock(&self.state);
            if s.direct == Some(true) {
                s.direct = Some(false);
                s.segments = None;
                s.final_url = None;
                s.size = None;
                let _ = std::fs::remove_file(self.part_path_of(&s));
            }
            // yt-dlp counts what an earlier run already left on disk, so every run starts its own tally from zero
            // (a base kept from a paused run would be added on top and inflate the size).
            s.ytd = Some(Ytd::default());
            s.frag = None;
            // The chosen name is fixed once, so a resumed run finds the same partial file and a finished one is
            // never overwritten.
            if s.out_stem.as_deref().is_none_or(str::is_empty)
                && let Some(name) = s.out_name.clone().filter(|n| !n.is_empty())
            {
                let free = unique_path(Path::new(&s.dir), &format!("{name}.mp4"), false);
                let file = free
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                s.out_stem = Some(file.strip_suffix(".mp4").unwrap_or(&file).to_string());
            }
        }
        self.clear_samples();
        let started_at = now_ms();
        let ffmpeg = self.ctx.ffmpeg();
        let args = {
            let s = lock(&self.state);
            ytdlp_args(
                &s,
                &self.ctx.settings(),
                self.ctx
                    .limit_kbps
                    .load(std::sync::atomic::Ordering::Relaxed),
                ffmpeg.as_deref(),
                self.ctx.deno().as_deref(),
            )
        };

        let mut cmd = yt.command(&args);
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| TaskError::from_io(&e))?;
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        if let Some(o) = child.stdout.take() {
            pump_lines(o, tx.clone());
        }
        if let Some(e) = child.stderr.take() {
            pump_lines(e, tx.clone());
        }
        drop(tx);
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

        let mut out = Out {
            first_report: true,
            final_file: None,
            last_err: String::new(),
            lines: vec![],
            last_progress: String::new(),
            tail: vec![],
        };
        let mut killed = false;
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled(), if !killed => {
                    killed = true;
                    kill_tree(&mut child).await;
                }
                line = rx.recv() => match line {
                    Some(l) => self.on_ytdlp_line(&mut out, &l, ffmpeg.is_some()),
                    None => break,
                },
            }
        }
        let status = child.wait().await;
        if cancel.is_cancelled() {
            return Err(Fail::Cancelled);
        }
        let code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

        let found = |f: &Option<String>| f.as_deref().is_some_and(|f| Path::new(f).exists());
        // The printed path can still differ from the real one (characters lost on the way): find the file by the
        // video id in its name.
        if code == 0
            && !found(&out.final_file)
            && let Some(id) = video_id(out.final_file.as_deref(), &url)
        {
            let where_ = out
                .final_file
                .as_deref()
                .and_then(|f| Path::new(f).parent())
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from(&dir), Path::to_path_buf);
            if let Some(real) = find_by_video_id(&where_, &id, started_at.saturating_sub(5000)) {
                out.final_file = Some(real.to_string_lossy().into_owned());
            }
        }
        if code != 0 || !found(&out.final_file) {
            let detail = if out.last_err.is_empty() {
                out.tail.join(" · ")
            } else {
                out.last_err.clone()
            };
            let message = explain(&detail, &url).unwrap_or_else(|| {
                if detail.is_empty() {
                    format!("yt-dlp n’a pas pu télécharger cette vidéo (code {code})")
                } else {
                    detail.clone()
                }
            });
            self.write_diag(&url, &args, code, &out, &message);
            let mut failure = TaskError::fatal(message);
            failure.retryable = is_retryable(&detail);
            return Err(Fail::Error(failure));
        }
        self.write_diag(&url, &args, code, &out, "");
        // yt-dlp already wrote it in place: only a move to another folder needs a free name.
        let file = PathBuf::from(out.final_file.as_deref().unwrap_or_default());
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        {
            let mut s = lock(&self.state);
            let target = settle_finished(&self.ctx, &mut s, &file, &name)?;
            let size = std::fs::metadata(&target)?.len();
            s.size = Some(size);
            s.ytd = Some(Ytd {
                base: 0.0,
                cur: size as f64,
                total: 0.0,
            });
            s.frag = None;
        }
        self.set(Status::Completed, None);
        Ok(())
    }

    fn on_ytdlp_line(&self, o: &mut Out, line: &str, has_ffmpeg: bool) {
        if let Some(p) = line.strip_prefix("DMP ") {
            o.last_progress = p.to_string();
        } else if !line.is_empty() {
            o.lines.push(line.to_string());
            if o.lines.len() > 300 {
                o.lines.remove(0);
            }
        }
        if line.starts_with("DMP ") {
            let gained = apply_progress(&mut lock(&self.state), line, o.first_report);
            o.first_report = false;
            if gained > 0 {
                lock(&self.samples).push_back((Instant::now(), gained));
            }
        } else if let Some(title) = line.strip_prefix("DMT ") {
            let mut s = lock(&self.state);
            if s.out_stem.as_deref().is_none_or(str::is_empty) {
                let title = title.trim();
                let ext = if YtOpts::from_value(s.yt_opts.as_ref()).audio_only {
                    if has_ffmpeg { ".mp3" } else { ".m4a" }
                } else {
                    ".mp4"
                };
                s.filename = format!(
                    "{}{ext}",
                    sanitize_filename(if title.is_empty() { "video" } else { title })
                );
            }
        } else if let Some(f) = line.strip_prefix("DMF ") {
            o.final_file = Some(f.trim().to_string());
        } else if let Some(e) = error_text(line) {
            o.last_err = e;
        } else if !line.is_empty() && !line.starts_with("[download]") {
            o.tail.push(warning_text(line));
            if o.tail.len() > 4 {
                o.tail.remove(0);
            }
        }
    }

    /// The diagnostic of the last run, a courtesy: never fail the download over it.
    fn write_diag(&self, url: &str, args: &[String], code: i32, o: &Out, error: &str) {
        let Some(file) = self.ctx.diag_file() else {
            return;
        };
        let text = format_diag(&Diag {
            url,
            args,
            code,
            final_file: o.final_file.as_deref(),
            file_found: o
                .final_file
                .as_deref()
                .is_some_and(|f| Path::new(f).exists()),
            error,
            lines: &o.lines,
            last_progress: &o.last_progress,
            at: chrono::Utc::now(),
        });
        let _ = std::fs::write(file, text);
    }

    /// Asks yt-dlp which file it would download; the plain file behind the video, or none.
    async fn resolve_direct(
        &self,
        yt: &YtdlpBin,
        cancel: &CancellationToken,
    ) -> Option<DirectFile> {
        let cfg = self.ctx.settings();
        let (url, headers, opts) = {
            let s = lock(&self.state);
            (
                s.url.clone(),
                s.headers.clone(),
                YtOpts::from_value(s.yt_opts.as_ref()),
            )
        };
        let max_height = if cfg.video_max_height > 0 {
            cfg.video_max_height
        } else {
            1080
        };
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
        args.extend(format_args(&opts, max_height, self.ctx.ffmpeg().is_some()));
        if let Some(d) = self.ctx.deno() {
            args.push("--js-runtimes".into());
            args.push(format!("deno:{}", d.display()));
        }
        if !cfg.proxy.is_empty() {
            args.push("--proxy".into());
            args.push(cfg.proxy.clone());
        }
        if !cfg.yt_cookies.is_empty() {
            args.push("--cookies-from-browser".into());
            args.push(cfg.yt_cookies.clone());
        }
        args.extend(header_args(headers.as_ref(), &url));
        args.push(url);
        let mut cmd = yt.command(&args);
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let out = tokio::select! {
            _ = cancel.cancelled() => return None,
            r = tokio::time::timeout(Duration::from_secs(60), cmd.output()) => r.ok()?.ok()?,
        };
        if !out.status.success() {
            return None;
        }
        serde_json::from_slice(&out.stdout)
            .ok()
            .and_then(|v| direct_file(&v))
    }

    /// Downloads the plain file over several connections with the app's own engine. False when it cannot be done
    /// that way.
    async fn run_direct(
        self: &Arc<Self>,
        direct: &DirectFile,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        {
            let mut s = lock(&self.state);
            let page = s.url.clone();
            let h = s.headers.get_or_insert_with(Default::default);
            h.referer = page;
            h.origin.clear();
            h.user_agent = direct.user_agent.clone();
        }
        let headers = self.base_headers(&direct.url);
        let info = probe(&self.ctx.net(), &direct.url, &headers, cancel).await?;
        if !info.range_ok || info.size.is_none_or(|n| n == 0) {
            return Ok(false);
        }
        {
            let mut s = lock(&self.state);
            if s.segments.is_none() && s.replaces.is_none() {
                // taken and reserved in one go, like any other download
                let _naming = lock(&self.ctx.naming);
                std::fs::create_dir_all(&s.dir)?;
                let wanted = format!("{}.{}", sanitize_filename(&direct.title), direct.ext);
                let free = unique_path(Path::new(&s.dir), &wanted, false);
                s.filename = free
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(wanted);
                std::fs::write(self.part_path_of(&s), b"")?;
            }
            s.final_url = Some(info.final_url);
            s.size = info.size;
            s.range_ok = true;
            s.etag = info.etag;
            s.last_modified = info.last_modified;
            s.direct = Some(true);
        }
        self.run_http(cancel.clone()).await?;
        Ok(true)
    }
}
