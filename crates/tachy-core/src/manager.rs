//! The download manager (`manager.js`): the list of downloads, the queue, the schedule, automatic retries and
//! the save to `downloads.json`. Cheap to clone; every clone is the same manager.

use crate::ctx::{Ctx, Event, Msg, SiteAuth, lock};
use crate::limiter::TokenBucket;
use crate::net::{Net, normalize_proxy};
use crate::settings::Settings;
use crate::state::{Kind, ReplayHeaders, Snapshot, Status, TaskState};
use crate::store;
use crate::task::Task;
use crate::util::{filename_from_url, sanitize_filename};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

/// Waits before each automatic retry of a download that lost its connection, in seconds.
pub const RETRY_DELAYS: [f64; 6] = [15.0, 30.0, 60.0, 120.0, 300.0, 600.0];

pub struct ManagerConfig {
    pub store_path: Option<PathBuf>,
    /// Settings to start from, merged over the defaults (`dir`, `connections`...).
    pub settings: Value,
    /// Cycle of the manager (1000 ms in the app, shorter in tests).
    pub tick_ms: u64,
    pub retry_delays: Vec<f64>,
    pub site_auth: Option<SiteAuth>,
    /// HLS: how many bytes may be received ahead of the segment being waited for (128 MiB when unset).
    pub hls_ahead_bytes: Option<u64>,
    /// Where the video tools are (yt-dlp, ffmpeg, deno).
    pub tools: Option<crate::ctx::ToolFinder>,
    /// How yt-dlp is started (only the tests change it, to run a stand-in script).
    pub ytdlp_launch: Option<crate::ctx::YtdlpLaunch>,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            store_path: None,
            settings: Value::Object(Default::default()),
            tick_ms: 1000,
            retry_delays: RETRY_DELAYS.to_vec(),
            site_auth: None,
            hls_ahead_bytes: None,
            tools: None,
            ytdlp_launch: None,
        }
    }
}

/// How a download is added (what the window and the extension can ask for).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AddOpts {
    pub kind: Option<String>,
    pub filename: Option<String>,
    pub dir: Option<String>,
    pub referer: Option<String>,
    pub origin: Option<String>,
    pub cookie: Option<String>,
    pub user_agent: Option<String>,
    pub page: Option<String>,
    pub sniffed: bool,
    pub start_at: Option<u64>,
    pub paused: bool,
    pub own_folder: bool,
    pub yt_opts: Option<Value>,
}

/// A fresh link for a download stopped by an expired one.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RefreshOpts {
    pub url: String,
    pub referer: Option<String>,
    pub origin: Option<String>,
    pub cookie: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshRequest {
    pub id: String,
    pub filename: String,
    pub size: Option<u64>,
    pub kind: Kind,
    pub page_url: String,
    pub auto: bool,
}

struct Inner {
    ctx: Arc<Ctx>,
    limiter: Arc<TokenBucket>,
    tasks: Mutex<Vec<Arc<Task>>>,
    store_path: Option<PathBuf>,
    retry_delays: Vec<f64>,
    tick_ms: u64,
    applied_limit: Mutex<Option<u64>>,
    shutdown: CancellationToken,
    rx: Mutex<Option<mpsc::UnboundedReceiver<Msg>>>,
}

#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Local time of day, in minutes since midnight.
pub fn local_minutes() -> u32 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    now.hour() * 60 + now.minute()
}

pub fn validate_url(url: &str) -> Result<String, String> {
    let u = url::Url::parse(url).map_err(|_| "Adresse invalide".to_string())?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return Err("Seuls les liens http et https sont pris en charge".into());
    }
    Ok(u.to_string())
}

fn is_hls_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        let p = u.path().to_ascii_lowercase();
        p.ends_with(".m3u8") || p.ends_with(".m3u")
    })
}

impl Manager {
    pub fn new(cfg: ManagerConfig) -> Manager {
        let settings = Settings::from_patch(&cfg.settings);
        let net = Net::new(&settings.proxy);
        let (tx, rx) = mpsc::unbounded_channel();
        let ctx = Ctx::new(settings, net, cfg.site_auth, tx);
        if let Some(n) = cfg.hls_ahead_bytes {
            ctx.hls_ahead_bytes
                .store(n, std::sync::atomic::Ordering::Relaxed);
        }
        *ctx.tools.write().unwrap_or_else(|e| e.into_inner()) = cfg.tools;
        if let Some(l) = cfg.ytdlp_launch {
            *ctx.ytdlp_launch.write().unwrap_or_else(|e| e.into_inner()) = l;
        }
        *ctx.diag_file.write().unwrap_or_else(|e| e.into_inner()) = cfg
            .store_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.join("ytdlp-last.log"));
        let m = Manager {
            inner: Arc::new(Inner {
                ctx,
                limiter: Arc::new(TokenBucket::new(0.0)),
                tasks: Mutex::new(Vec::new()),
                store_path: cfg.store_path,
                retry_delays: cfg.retry_delays,
                tick_ms: cfg.tick_ms.max(1),
                applied_limit: Mutex::new(None),
                shutdown: CancellationToken::new(),
                rx: Mutex::new(Some(rx)),
            }),
        };
        m.sync_limit();
        m
    }

    pub fn settings(&self) -> Settings {
        self.inner.ctx.settings()
    }

    /// Tells the downloads where ffmpeg is (it appears once the video tools are installed).
    pub fn set_ffmpeg(&self, f: crate::ctx::FfmpegPath) {
        *self
            .inner
            .ctx
            .ffmpeg
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// Tells the downloads where the video tools are (they appear once installed).
    pub fn set_tools(&self, f: crate::ctx::ToolFinder) {
        *self
            .inner
            .ctx
            .tools
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// Changes how yt-dlp is started (the tests run a stand-in script).
    pub fn set_ytdlp_launch(&self, l: crate::ctx::YtdlpLaunch) {
        *self
            .inner
            .ctx
            .ytdlp_launch
            .write()
            .unwrap_or_else(|e| e.into_inner()) = l;
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.ctx.events.subscribe()
    }

    /// The shared bandwidth limiter (a rate of 0 is unlimited).
    pub fn limiter(&self) -> &TokenBucket {
        &self.inner.limiter
    }

    pub fn task(&self, id: &str) -> Option<Arc<Task>> {
        lock(&self.inner.tasks).iter().find(|t| t.id == id).cloned()
    }

    fn get(&self, id: &str) -> Result<Arc<Task>, String> {
        self.task(id)
            .ok_or_else(|| "Téléchargement introuvable".to_string())
    }

    fn all(&self) -> Vec<Arc<Task>> {
        lock(&self.inner.tasks).clone()
    }

    pub fn list(&self) -> Vec<Snapshot> {
        self.all().iter().map(|t| t.snapshot()).collect()
    }

    // ---- settings ---------------------------------------------------------------------------------------

    /// The limit in force now (KB/s): the fixed one, tightened by the time-of-day one while its window is open.
    pub fn effective_limit(&self, minutes_now: u32) -> u64 {
        self.settings().effective_limit(minutes_now)
    }

    /// Pushes the limit in force to the shared bucket, only when it changed (setting a rate refills the bucket).
    fn sync_limit(&self) {
        let kbps = self.effective_limit(local_minutes());
        let mut applied = lock(&self.inner.applied_limit);
        if *applied == Some(kbps) {
            return;
        }
        *applied = Some(kbps);
        self.inner.limiter.set_rate(kbps as f64 * 1024.0);
        self.inner
            .ctx
            .limit_kbps
            .store(kbps, std::sync::atomic::Ordering::Relaxed);
    }

    /// Downloads go through the proxy when one is set.
    fn apply_net(&self) {
        let proxy = self.settings().proxy;
        if self.inner.ctx.net().proxy() != proxy {
            *self
                .inner
                .ctx
                .net
                .write()
                .unwrap_or_else(|e| e.into_inner()) = Net::new(&proxy);
        }
    }

    pub fn set_settings(&self, patch: &Value) -> Result<Settings, String> {
        if let Some(p) = patch.get("proxy") {
            // refused when it cannot work; the old one stays
            let typed = match p {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            normalize_proxy(&typed)?;
        }
        let next = self.settings().merged(patch);
        *self
            .inner
            .ctx
            .settings
            .write()
            .unwrap_or_else(|e| e.into_inner()) = next.clone();
        self.sync_limit();
        self.apply_net();
        self.inner.ctx.dirty.notify_one();
        let _ = self
            .inner
            .ctx
            .events
            .send(Event::Settings(Box::new(next.clone())));
        self.tick();
        Ok(next)
    }

    // ---- loading, saving --------------------------------------------------------------------------------

    pub fn load(&self) {
        let Some(path) = &self.inner.store_path else {
            return;
        };
        let Some(data) = store::load(path) else {
            return;
        };
        if let Some(s) = data.settings {
            *self
                .inner
                .ctx
                .settings
                .write()
                .unwrap_or_else(|e| e.into_inner()) = self.settings().merged(&s);
            self.sync_limit();
            self.apply_net();
        }
        for mut state in data.tasks {
            // Anything that was in flight when the app closed waits for the user.
            if state.status == Status::Downloading {
                state.status = Status::Paused;
            }
            // A finished download never needs its session cookie again (older versions kept it on disk).
            if state.status == Status::Completed
                && let Some(h) = &mut state.headers
            {
                h.cookie.clear();
            }
            self.register(state);
        }
    }

    pub fn save_now(&self) {
        let Some(path) = &self.inner.store_path else {
            return;
        };
        let states: Vec<TaskState> = self.all().iter().map(|t| lock(&t.state).clone()).collect();
        let _ = store::save(path, &self.settings(), &states);
    }

    fn register(&self, state: TaskState) -> Arc<Task> {
        let task = Task::new(state, self.inner.limiter.clone(), self.inner.ctx.clone());
        lock(&self.inner.tasks).push(task.clone());
        task
    }

    // ---- life -------------------------------------------------------------------------------------------

    /// Starts the cycle (queue, schedule, retries) and the delayed save. Needs a Tokio runtime.
    pub fn start(&self) {
        let rx = lock(&self.inner.rx).take();
        if let Some(mut rx) = rx {
            let me = self.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_millis(me.inner.tick_ms));
                loop {
                    tokio::select! {
                        _ = me.inner.shutdown.cancelled() => break,
                        _ = tick.tick() => me.tick(),
                        msg = rx.recv() => match msg {
                            Some(Msg::Settled(id)) => me.on_settled(&id),
                            None => break,
                        },
                    }
                }
            });
            let me = self.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = me.inner.shutdown.cancelled() => break,
                        _ = me.inner.ctx.dirty.notified() => {
                            tokio::select! {
                                _ = tokio::time::sleep(Duration::from_millis(1000)) => me.save_now(),
                                _ = me.inner.shutdown.cancelled() => break,
                            }
                        }
                    }
                }
            });
        }
        self.tick();
    }

    pub async fn shutdown(&self) {
        self.inner.shutdown.cancel();
        let stops = self.all().into_iter().map(|t| async move {
            let status = t.status();
            t.stop(if status == Status::Downloading {
                Status::Paused
            } else {
                status
            })
            .await;
        });
        futures_util::future::join_all(stops).await;
        self.save_now();
    }

    fn on_settled(&self, id: &str) {
        if let Some(t) = self.task(id) {
            self.plan_retry(&t);
        }
        self.pump();
    }

    // ---- adding and changing downloads -------------------------------------------------------------------

    pub fn add(&self, url: &str, opts: &AddOpts) -> Result<Snapshot, String> {
        let clean = validate_url(url)?;
        let kind = match opts.kind.as_deref() {
            Some("ytdlp") => Kind::Ytdlp,
            Some("hls") => Kind::Hls,
            None if crate::video::is_video_page(&clean) => Kind::Ytdlp,
            _ if is_hls_url(&clean) => Kind::Hls,
            _ => Kind::File,
        };
        let is_hls = kind == Kind::Hls;
        let settings = self.settings();
        let text = |o: &Option<String>| o.clone().unwrap_or_default();
        let mut state = TaskState {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            own_folder: opts.own_folder,
            // The page to reopen for a fresh link. The Referer is what the browser sent with the request, which can be
            // the player's frame rather than the page: without a separate page, the Referer stands in.
            page_url: [&opts.page, &opts.referer]
                .iter()
                .filter_map(|u| u.as_deref())
                .find(|u| {
                    u.to_ascii_lowercase().starts_with("http://")
                        || u.to_ascii_lowercase().starts_with("https://")
                })
                .unwrap_or("")
                .to_string(),
            auto_refresh: opts.sniffed, // a video the extension found: it can reopen the page itself
            url: clean.clone(),
            dir: opts
                .dir
                .clone()
                .filter(|d| !d.is_empty())
                .unwrap_or(settings.dir.clone()),
            filename_locked: opts.filename.as_deref().is_some_and(|f| !f.is_empty()),
            connections: settings.connections,
            headers: Some(ReplayHeaders {
                referer: text(&opts.referer),
                origin: text(&opts.origin),
                // yt-dlp reads the cookies of the chosen browser itself: nothing to keep for it.
                cookie: if kind == Kind::Ytdlp {
                    String::new()
                } else {
                    text(&opts.cookie)
                },
                user_agent: text(&opts.user_agent),
            }),
            status: Status::Queued,
            created_at: now_ms(),
            start_at: opts.start_at.filter(|n| *n > 0),
            ..TaskState::default()
        };
        // Placeholder until the first request; the server's own name replaces it unless one was given.
        let wanted = opts
            .filename
            .clone()
            .filter(|f| !f.is_empty())
            .or_else(|| {
                if kind == Kind::Ytdlp {
                    Some("Vidéo".to_string())
                } else {
                    filename_from_url(&clean)
                }
                .filter(|f| !f.is_empty())
            })
            .unwrap_or_else(|| {
                if is_hls {
                    "video".into()
                } else {
                    "download".into()
                }
            });
        state.filename = sanitize_filename(&wanted);
        if kind == Kind::Ytdlp {
            state.yt_opts = opts.yt_opts.as_ref().and_then(crate::video::clean_yt_opts);
            // A manifest the extension found has no title of its own ("manifest"): the file takes the name of the page.
            if opts.sniffed
                && let Some(f) = opts.filename.as_deref().filter(|f| !f.is_empty())
            {
                state.out_name = Some(sanitize_filename(f));
            }
        }
        if state.dir.is_empty() {
            return Err("Aucun dossier de destination configuré".into());
        }
        state.filename = self.free_name(&state.dir, &state.filename, is_hls);
        if state.start_at.is_some_and(|t| t > now_ms()) {
            state.status = Status::Scheduled;
        } else if opts.paused {
            state.status = Status::Paused; // "later": waits in the list until the user resumes it
        }
        let task = self.register(state);
        task.emit();
        self.pump();
        Ok(task.snapshot())
    }

    /// Adds every video of a YouTube playlist, in its own folder named after the playlist.
    pub async fn add_playlist(&self, url: &str, opts: &AddOpts) -> Result<Vec<Snapshot>, String> {
        let Some(yt) = self.inner.ctx.ytdlp() else {
            return Err("Outils vidéo non installés : ouvrez Paramètres → Vidéos".into());
        };
        let settings = self.settings();
        let list =
            crate::video::list_playlist(&yt, &validate_url(url)?, 200, &settings.proxy).await?;
        if list.items.is_empty() {
            return Err("Cette playlist est vide ou privée".into());
        }
        let base = opts
            .dir
            .clone()
            .filter(|d| !d.is_empty())
            .unwrap_or(settings.dir);
        let dir = Path::new(&base)
            .join(sanitize_filename(&list.title))
            .to_string_lossy()
            .into_owned();
        list.items
            .iter()
            .map(|it| {
                self.add(
                    &it.url,
                    &AddOpts {
                        kind: Some("ytdlp".into()),
                        dir: Some(dir.clone()),
                        filename: (!it.title.is_empty()).then(|| format!("{}.mp4", it.title)),
                        own_folder: true,
                        ..opts.clone()
                    },
                )
            })
            .collect()
    }

    /// Two unfinished downloads must never share a .part file, so a repeated name gets a suffix.
    fn free_name(&self, dir: &str, filename: &str, is_hls: bool) -> String {
        let stem = |n: &str| -> String {
            let n = if is_hls {
                let lower = n.to_ascii_lowercase();
                [".m3u8", ".m3u", ".ts", ".mp4"]
                    .iter()
                    .find(|e| lower.ends_with(*e))
                    .map_or(n.to_string(), |e| n[..n.len() - e.len()].to_string())
            } else {
                n.to_string()
            };
            stem_lower(&n)
        };
        let taken: std::collections::HashSet<String> = self
            .all()
            .iter()
            .filter_map(|t| {
                let s = lock(&t.state);
                (s.status != Status::Completed && s.dir == dir).then(|| stem(&s.filename))
            })
            .collect();
        if !taken.contains(&stem(filename)) {
            return filename.to_string();
        }
        let ext = crate::util::extname(filename);
        let base = &filename[..filename.len() - ext.len()];
        for i in 1.. {
            let candidate = format!("{base} ({i}){ext}");
            if !taken.contains(&stem(&candidate)) {
                return candidate;
            }
        }
        unreachable!()
    }

    /// Downloads stopped by an expired link, for the extension to find a fresh one.
    pub fn refresh_requests(&self) -> Vec<RefreshRequest> {
        self.all()
            .iter()
            .filter_map(|t| {
                let s = lock(&t.state);
                (s.needs_refresh && s.status == Status::Error).then(|| RefreshRequest {
                    id: s.id.clone(),
                    filename: s.filename.clone(),
                    size: s.size,
                    kind: s.kind,
                    page_url: s.page_url.clone(),
                    auto: s.auto_refresh,
                })
            })
            .collect()
    }

    /// Swaps in a fresh link for the same file; what is already on disk is kept and the transfer resumes.
    pub fn refresh(&self, id: &str, opts: &RefreshOpts) -> Result<Snapshot, String> {
        let t = self.get(id)?;
        if t.is_active() || t.status() == Status::Completed {
            return Err("Ce téléchargement n’a pas besoin d’un nouveau lien".into());
        }
        let url = validate_url(&opts.url)?;
        {
            let mut s = lock(&t.state);
            s.url = url;
            s.final_url = None; // forces a new probe on the new address
            s.etag = None;
            s.last_modified = None;
            s.filename_locked = true; // the .part file already carries its final name
            let h = s.headers.get_or_insert_with(Default::default);
            for (slot, v) in [
                (&mut h.referer, &opts.referer),
                (&mut h.origin, &opts.origin),
                (&mut h.cookie, &opts.cookie),
                (&mut h.user_agent, &opts.user_agent),
            ] {
                if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
                    *slot = v.clone();
                }
            }
            s.needs_refresh = false;
            s.error = None;
            s.status = Status::Queued;
        }
        t.emit();
        self.pump();
        Ok(t.snapshot())
    }

    /// Downloads a finished file again, in the same row and under the same name. The old file stays where it is
    /// until the new one is complete and then gives way to it: a failure (an expired link, say) loses nothing.
    pub fn redownload(&self, id: &str) -> Result<Snapshot, String> {
        let t = self.get(id)?;
        let old = {
            let s = lock(&t.state);
            if s.status != Status::Completed {
                return Err("Seul un téléchargement terminé peut être refait".into());
            }
            Path::new(&s.dir).join(&s.filename)
        };
        let mut replaces = None;
        if old.exists() {
            // fails when another program holds the file or it is read-only
            if std::fs::OpenOptions::new().write(true).open(&old).is_err() {
                return Err("Le fichier ne peut pas être remplacé : fermez-le dans l’autre programme qui l’utilise (ou retirez la lecture seule), puis réessayez.".into());
            }
            replaces = Some(old.to_string_lossy().into_owned());
        }
        {
            let mut s = lock(&t.state);
            s.replaces = replaces;
            s.status = Status::Queued;
            s.error = None;
            s.completed_at = None;
            s.start_at = None;
            s.retry_at = None;
            s.retry_count = Some(0);
            s.needs_refresh = false;
            s.filename_locked = true; // keep the name: the old file is the one in the way, not a reason for "(1)"
            s.own_folder = true; // it stays in the folder where the old file is, whatever the sorting rules say
            s.size = None;
            s.final_url = None;
            s.range_ok = false;
            s.etag = None;
            s.last_modified = None;
            s.segments = None;
            s.frag = None;
            s.phase = None;
            s.direct = Some(false);
            s.hls = None;
            s.ytd = None;
        }
        t.clear_samples();
        t.emit();
        self.pump();
        Ok(t.snapshot())
    }

    pub async fn pause(&self, id: &str) -> Result<(), String> {
        let t = self.get(id)?;
        if t.status() == Status::Completed {
            return Ok(());
        }
        lock(&t.state).retry_at = None;
        t.stop(Status::Paused).await;
        Ok(())
    }

    pub fn resume(&self, id: &str) -> Result<(), String> {
        let t = self.get(id)?;
        if matches!(
            t.status(),
            Status::Paused | Status::Error | Status::Scheduled
        ) {
            {
                let mut s = lock(&t.state);
                s.status = Status::Queued;
                s.error = None;
                s.start_at = None;
                s.retry_at = None; // a manual restart starts the automatic attempts over
                s.retry_count = Some(0);
            }
            t.emit();
            self.pump();
        }
        Ok(())
    }

    /// Own speed limit of one download, in KB/s (0 removes it).
    pub fn set_task_limit(&self, id: &str, kbps: f64) -> Result<Snapshot, String> {
        let t = self.get(id)?;
        t.set_limit(kbps);
        Ok(t.snapshot())
    }

    /// Takes every finished download off the list (the files stay where they are). Returns how many.
    pub async fn clear_completed(&self) -> usize {
        let ids: Vec<String> = self
            .all()
            .iter()
            .filter(|t| t.status() == Status::Completed)
            .map(|t| t.id.clone())
            .collect();
        for id in &ids {
            let _ = self.remove(id, false).await;
        }
        ids.len()
    }

    pub async fn remove(&self, id: &str, delete_file: bool) -> Result<(), String> {
        let t = self.get(id)?;
        t.stop(Status::Paused).await;
        lock(&self.inner.tasks).retain(|x| x.id != id);
        if delete_file {
            let (completed, target) = {
                let s = lock(&t.state);
                (
                    s.status == Status::Completed,
                    Path::new(&s.dir).join(&s.filename),
                )
            };
            let _ = std::fs::remove_file(if completed { target } else { t.part_path() });
            let _ = std::fs::remove_file(t.part_path());
        }
        let _ = self.inner.ctx.events.send(Event::Removed(id.to_string()));
        self.inner.ctx.dirty.notify_one();
        self.pump();
        Ok(())
    }

    // ---- the cycle --------------------------------------------------------------------------------------

    /// A download that failed on a dropped connection tries again by itself, a little later each time. The count
    /// starts over when it made headway since the last attempt, so a long download on a bad line keeps going.
    fn plan_retry(&self, t: &Arc<Task>) {
        let auto_retry = self.settings().auto_retry;
        let got = t.downloaded();
        {
            let mut s = lock(&t.state);
            if s.status != Status::Error
                || s.retryable != Some(true)
                || s.needs_refresh
                || !auto_retry
            {
                s.retry_at = None;
                return;
            }
            if s.retry_bytes.is_none_or(|b| got > b) {
                s.retry_count = Some(0);
            }
            s.retry_bytes = Some(got);
            let n = s.retry_count.unwrap_or(0) as usize;
            if n >= self.inner.retry_delays.len() {
                s.retry_at = None;
                return;
            }
            s.retry_count = Some(n as u32 + 1);
            s.retry_at = Some(now_ms() + (self.inner.retry_delays[n] * 1000.0) as u64);
        }
        t.emit();
    }

    pub fn tick(&self) {
        self.sync_limit();
        let now = now_ms();
        let settings = self.settings();
        let minutes = local_minutes();
        let in_window = settings.in_schedule_window(minutes);
        for t in self.all() {
            let mut changed = false;
            {
                let mut s = lock(&t.state);
                if s.status == Status::Error && s.retry_at.is_some_and(|r| r <= now) && in_window {
                    s.retry_at = None;
                    s.status = Status::Queued;
                    s.error = None;
                    changed = true;
                }
                if s.status == Status::Scheduled && s.start_at.is_some_and(|a| a <= now) {
                    s.status = Status::Queued;
                    changed = true;
                }
            }
            if changed {
                t.emit();
            }
        }
        if !in_window {
            for t in self.all() {
                if t.is_active() {
                    let t = t.clone();
                    tokio::spawn(async move { t.stop(Status::Queued).await });
                }
            }
        }
        self.pump();
    }

    pub fn pump(&self) {
        let settings = self.settings();
        if !settings.in_schedule_window(local_minutes()) {
            return;
        }
        let tasks = self.all();
        let mut active = tasks.iter().filter(|t| t.is_active()).count();
        for t in tasks {
            if active >= settings.max_concurrent as usize {
                break;
            }
            if t.status() == Status::Queued && !t.is_active() {
                t.start();
                active += 1;
            }
        }
    }
}

fn stem_lower(s: &str) -> String {
    s.to_lowercase()
}
