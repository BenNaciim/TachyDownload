//! One download (`download.js`): the transfer of a plain file over several connections, with resume, work
//! stealing between segments, retries, adaptive connection count and a speed limit. Streams (HLS) and
//! yt-dlp downloads plug into the same `Task` in later milestones.

use crate::ctx::{Ctx, Event, Msg, lock};
use crate::errors::{Fail, Result, TaskError};
use crate::fsutil::{settle_finished, write_all_at};
use crate::limiter::TokenBucket;
use crate::net::Headers;
use crate::probe::probe;
use crate::state::{Kind, Segment, Snapshot, Status, TaskState, UNKNOWN_END};
use crate::util::unique_path;
use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

mod hls;

pub const MIN_SEGMENT: u64 = 1024 * 1024;
/// Only a segment with more than this left is worth splitting.
pub const MIN_STEAL: u64 = 2 * 1024 * 1024;
pub const MAX_RETRIES: u32 = 5;
const SPEED_WINDOW: Duration = Duration::from_millis(3000);
/// Bytes gathered before one write: fewer system calls (the kernel time is half of the CPU of a fast download).
const BLOCK: usize = 1024 * 1024;
/// Whatever has arrived is written at least this often, so progress shows on a slow link.
const FLUSH_EVERY: Duration = Duration::from_millis(100);

/// Equal steps over the file, the last one taking the rest; never a segment under 1 MiB.
pub fn split_segments(size: u64, count: u32) -> Vec<Segment> {
    let n = u64::from(count).min(size.div_ceil(MIN_SEGMENT)).max(1);
    let step = size / n;
    (0..n)
        .map(|i| {
            let start = i * step;
            let end = if i == n - 1 {
                size.saturating_sub(1)
            } else {
                start + step - 1
            };
            Segment {
                start,
                end,
                pos: start,
            }
        })
        .collect()
}

fn set_header(h: &mut Headers, name: &str, value: &str) {
    h.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    h.push((name.to_string(), value.to_string()));
}

/// Headers replayed with a request: the saved login of the site, then what the browser sent.
fn replay_headers(ctx: &Ctx, s: &TaskState, url: &str, if_range: bool) -> Headers {
    let mut h = ctx.site_headers(url);
    if let Some(r) = &s.headers {
        for (name, v) in [
            ("Referer", &r.referer),
            ("Origin", &r.origin),
            ("Cookie", &r.cookie),
            ("User-Agent", &r.user_agent),
        ] {
            if !v.is_empty() {
                set_header(&mut h, name, v);
            }
        }
    }
    if if_range {
        match (&s.etag, &s.last_modified) {
            (Some(e), _) if !e.starts_with("W/") => set_header(&mut h, "If-Range", e),
            (_, Some(m)) if !m.is_empty() => set_header(&mut h, "If-Range", m),
            _ => {}
        }
    }
    h
}

struct RunInfo {
    active: bool,
    cancel: Option<CancellationToken>,
    stop_status: Status,
}

struct Conc {
    allowed: usize,
    workers: usize,
    last_throttle: Instant,
}

struct AbortOnDrop(JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct Task {
    pub id: String,
    pub state: std::sync::Mutex<TaskState>,
    ctx: Arc<Ctx>,
    limiter: Arc<TokenBucket>,
    /// This download's own ceiling, on top of the shared one.
    own: TokenBucket,
    run: std::sync::Mutex<RunInfo>,
    settled: Notify,
    samples: std::sync::Mutex<VecDeque<(Instant, u64)>>,
    conc: std::sync::Mutex<Conc>,
    wake: Notify,
    /// HLS: segments in flight and received ahead, for the details view.
    hls_live: std::sync::Mutex<Option<hls::Live>>,
}

impl Task {
    pub fn new(state: TaskState, limiter: Arc<TokenBucket>, ctx: Arc<Ctx>) -> Arc<Self> {
        let own = TokenBucket::new(state.speed_limit_kbps.unwrap_or(0) as f64 * 1024.0);
        Arc::new(Self {
            id: state.id.clone(),
            state: std::sync::Mutex::new(state),
            ctx,
            limiter,
            own,
            run: std::sync::Mutex::new(RunInfo {
                active: false,
                cancel: None,
                stop_status: Status::Paused,
            }),
            settled: Notify::new(),
            samples: std::sync::Mutex::new(VecDeque::new()),
            conc: std::sync::Mutex::new(Conc {
                allowed: usize::MAX,
                workers: 0,
                last_throttle: Instant::now(),
            }),
            wake: Notify::new(),
            hls_live: std::sync::Mutex::new(None),
        })
    }

    pub fn is_active(&self) -> bool {
        lock(&self.run).active
    }

    pub fn status(&self) -> Status {
        lock(&self.state).status
    }

    pub fn downloaded(&self) -> u64 {
        lock(&self.state).downloaded()
    }

    /// Connections allowed at once: starts at the configured number and drops when the server pushes back.
    pub fn allowed(&self) -> usize {
        lock(&self.conc).allowed
    }

    pub fn part_path(&self) -> PathBuf {
        let s = lock(&self.state);
        PathBuf::from(&s.dir).join(format!("{}.part", s.filename))
    }

    fn speed(&self) -> f64 {
        let mut samples = lock(&self.samples);
        let now = Instant::now();
        while samples
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > SPEED_WINDOW)
        {
            samples.pop_front();
        }
        let Some((first, _)) = samples.front() else {
            return 0.0;
        };
        let bytes: u64 = samples.iter().map(|(_, n)| n).sum();
        let span = now.duration_since(*first).as_secs_f64().max(1.0);
        bytes as f64 / span
    }

    pub fn clear_samples(&self) {
        lock(&self.samples).clear();
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut state = lock(&self.state).clone();
        let downloaded = state.downloaded();
        let speed = if state.status == Status::Downloading {
            self.speed()
        } else {
            0.0
        };
        let eta = match state.size {
            Some(size) if size > 0 && speed > 0.0 => {
                Some((size.saturating_sub(downloaded) as f64 / speed).ceil() as u64)
            }
            _ => None,
        };
        if let Some(h) = &mut state.headers {
            h.cookie.clear(); // the window has no use for the cookie
        }
        let mut more = serde_json::Map::new();
        if state.kind == Kind::Hls {
            self.hls_snapshot(&mut state, downloaded, &mut more);
        }
        Snapshot {
            state,
            downloaded,
            speed,
            eta,
            more,
        }
    }

    /// Tells the listeners and asks for a save.
    pub fn emit(&self) {
        let _ = self
            .ctx
            .events
            .send(Event::Update(Box::new(self.snapshot())));
        self.ctx.dirty.notify_one();
    }

    pub fn set(&self, status: Status, error: Option<String>) {
        {
            let mut s = lock(&self.state);
            s.status = status;
            if status == Status::Completed {
                if let Some(h) = &mut s.headers {
                    h.cookie.clear(); // not needed any more, do not keep it on disk
                }
                s.retry_at = None;
                s.retry_count = Some(0);
            }
            s.error = error;
        }
        self.emit();
    }

    /// Own speed limit in KB/s, 0 for none. Takes effect at once for plain files.
    pub fn set_limit(&self, kbps: f64) {
        let v = if kbps.is_finite() {
            kbps.round().max(0.0) as u64
        } else {
            0
        };
        lock(&self.state).speed_limit_kbps = Some(v);
        self.own.set_rate(v as f64 * 1024.0);
        self.emit();
    }

    // ---- running ----------------------------------------------------------------------------------------

    /// Starts (or resumes) the transfer in the background.
    pub fn start(self: &Arc<Self>) {
        let cancel = {
            let mut r = lock(&self.run);
            if r.active {
                return;
            }
            r.active = true;
            r.stop_status = Status::Paused;
            let c = CancellationToken::new();
            r.cancel = Some(c.clone());
            c
        };
        {
            let mut s = lock(&self.state);
            s.needs_refresh = false;
            s.retryable = Some(false);
        }
        self.set(Status::Downloading, None);
        let this = self.clone();
        tokio::spawn(async move {
            let inner = this.clone();
            let token = cancel.clone();
            // The transfer runs in a task of its own so that a panic ends this download only.
            let outcome = tokio::spawn(async move { inner.run(token).await }).await;
            this.finish_run(outcome, &cancel);
        });
    }

    fn finish_run(
        &self,
        outcome: std::result::Result<Result<()>, tokio::task::JoinError>,
        cancel: &CancellationToken,
    ) {
        let result = match outcome {
            Ok(r) => r,
            Err(join) => Err(Fail::Error(TaskError::new(format!(
                "Erreur interne : {join}"
            )))),
        };
        if let Err(fail) = result {
            if cancel.is_cancelled() {
                // stop() already showed the new status; a resume while the transfer was dying must not be undone.
                let next = lock(&self.run).stop_status;
                if self.status() == Status::Downloading {
                    self.set(next, None);
                }
            } else if let Fail::Error(err) = fail {
                if err.expired {
                    // Keeps the partial file: a fresh link for the same file resumes where it stopped.
                    lock(&self.state).needs_refresh = true;
                    let msg = if err.status == Some(401) {
                        "Connexion requise : ajoutez ce site dans Paramètres → Sites connectés"
                            .to_string()
                    } else {
                        format!("{} : le lien a peut-être expiré", err.message)
                    };
                    self.set(Status::Error, Some(msg));
                } else {
                    // A dropped connection is worth another try later; a refusal or a bad answer is not.
                    lock(&self.state).retryable = Some(!err.fatal || err.retryable);
                    self.set(Status::Error, Some(err.message));
                }
            }
        }
        {
            let mut r = lock(&self.run);
            r.active = false;
            r.cancel = None;
        }
        self.settled.notify_waiters();
        let _ = self.ctx.tx.send(Msg::Settled(self.id.clone()));
    }

    async fn wait_settled(&self) {
        loop {
            let notified = self.settled.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !lock(&self.run).active {
                return;
            }
            notified.await;
        }
    }

    /// Stops the transfer but keeps the partial file; `next` is the status left behind.
    pub async fn stop(&self, next: Status) {
        let cancel = {
            let mut r = lock(&self.run);
            if !r.active {
                drop(r);
                if self.status() != Status::Completed {
                    self.set(next, None);
                }
                return;
            }
            r.stop_status = next;
            r.cancel.clone()
        };
        if let Some(c) = cancel {
            c.cancel();
        }
        // Show it at once: killing a program and its children can take a while, and the click should answer now.
        self.set(next, None);
        self.wait_settled().await;
    }

    async fn run(self: &Arc<Self>, cancel: CancellationToken) -> Result<()> {
        let kind = lock(&self.state).kind;
        match kind {
            Kind::File => self.run_http(cancel).await,
            Kind::Hls => self.run_hls(cancel).await,
            Kind::Ytdlp => Err(Fail::Error(TaskError::fatal(
                "Ce type de téléchargement n’est pas encore pris en charge",
            ))),
        }
    }

    // ---- plain files ------------------------------------------------------------------------------------

    fn base_headers(&self, url: &str) -> Headers {
        replay_headers(&self.ctx, &lock(&self.state), url, false)
    }

    async fn prepare(&self, cancel: &CancellationToken) -> Result<()> {
        let (need_probe, url) = {
            let s = lock(&self.state);
            (s.size.is_none() || s.final_url.is_none(), s.url.clone())
        };
        if need_probe {
            let headers = self.base_headers(&url);
            let info = probe(&self.ctx.net(), &url, &headers, cancel).await?;
            let mut s = lock(&self.state);
            s.final_url = Some(info.final_url);
            s.size = info.size;
            s.range_ok = info.range_ok;
            s.etag = info.etag;
            s.last_modified = info.last_modified;
            if !s.filename_locked {
                // The name is taken and reserved (an empty .part file) in one go, so that two downloads of the
                // same name cannot both pick it.
                let _naming = lock(&self.ctx.naming);
                std::fs::create_dir_all(&s.dir)?;
                let free = unique_path(std::path::Path::new(&s.dir), &info.filename, false);
                s.filename = free
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(info.filename);
                std::fs::write(self.part_path_of(&s), b"")?;
            }
        }
        let (dir, part, size, range_ok, has_segments, connections) = {
            let s = lock(&self.state);
            (
                s.dir.clone(),
                self.part_path_of(&s),
                s.size,
                s.range_ok,
                s.segments.is_some(),
                s.connections,
            )
        };
        std::fs::create_dir_all(&dir)?;
        let resumable = has_segments
            && part.exists()
            && range_ok
            && size.is_some_and(|n| std::fs::metadata(&part).is_ok_and(|m| m.len() == n));
        if !resumable {
            lock(&self.state).segments = Some(match size {
                Some(n) if range_ok => split_segments(n, connections),
                Some(n) => vec![Segment {
                    start: 0,
                    end: n.saturating_sub(1),
                    pos: 0,
                }],
                None => vec![Segment {
                    start: 0,
                    end: UNKNOWN_END,
                    pos: 0,
                }],
            });
            self.clear_samples();
            std::fs::write(&part, b"")?;
            if let (Some(n), true) = (size, range_ok) {
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&part)?
                    .set_len(n)?;
            }
        }
        Ok(())
    }

    fn part_path_of(&self, s: &TaskState) -> PathBuf {
        PathBuf::from(&s.dir).join(format!("{}.part", s.filename))
    }

    async fn run_http(self: &Arc<Self>, cancel: CancellationToken) -> Result<()> {
        loop {
            self.prepare(&cancel).await?;
            let part = self.part_path();
            let (range_ok, connections) = {
                let s = lock(&self.state);
                (s.range_ok, s.connections)
            };
            let file = Arc::new(
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&part)?,
            );
            {
                let mut c = lock(&self.conc);
                c.allowed = (connections as usize).max(1);
                c.workers = 0;
                c.last_throttle = Instant::now();
            }
            let _ticker = {
                let this = self.clone();
                AbortOnDrop(tokio::spawn(async move {
                    let mut iv = tokio::time::interval_at(
                        tokio::time::Instant::now() + Duration::from_millis(250),
                        Duration::from_millis(250),
                    );
                    loop {
                        iv.tick().await;
                        this.speed_up();
                        this.emit();
                    }
                }))
            };
            // A connection that finishes its segment takes over half of the slowest one still running, so every
            // connection stays busy until the very end.
            let running = Arc::new(std::sync::Mutex::new(HashSet::<u64>::new()));
            let workers_cancel = cancel.child_token();
            let pending: Vec<u64> = lock(&self.state)
                .segments
                .as_ref()
                .map(|v| {
                    v.iter()
                        .filter(|g| g.pos <= g.end)
                        .map(|g| g.start)
                        .collect()
                })
                .unwrap_or_default();
            let mut set = JoinSet::new();
            for start in pending {
                set.spawn(self.clone().run_one(
                    start,
                    range_ok,
                    file.clone(),
                    workers_cancel.clone(),
                    running.clone(),
                ));
            }
            let mut first: Option<Fail> = None;
            while let Some(joined) = set.join_next().await {
                let r = joined.unwrap_or_else(|e| {
                    Err(Fail::Error(TaskError::new(format!("Erreur interne : {e}"))))
                });
                if let Err(f) = r
                    && first.is_none()
                {
                    first = Some(f);
                    workers_cancel.cancel(); // the others have no use going on
                }
            }
            if let Some(fail) = first {
                match fail {
                    Fail::Error(e) if e.changed && !cancel.is_cancelled() => {
                        // The remote file changed under us: the partial data is useless.
                        let mut s = lock(&self.state);
                        s.segments = None;
                        s.size = None;
                        s.final_url = None;
                        s.etag = None;
                        s.last_modified = None;
                        drop(s);
                        continue;
                    }
                    other => return Err(other),
                }
            }
            let (size, done) = {
                let s = lock(&self.state);
                (s.size, s.downloaded())
            };
            if size.is_some_and(|n| n > 0 && done != n) {
                return Err(Fail::Error(TaskError::new("Taille finale inattendue")));
            }
            let f = file.clone();
            tokio::task::spawn_blocking(move || f.sync_all())
                .await
                .map_err(|e| TaskError::new(e.to_string()))??;
            break;
        }
        if cancel.is_cancelled() {
            return Ok(());
        }
        // Our own .part file reserves the name, so only a finished file can collide here.
        let part = self.part_path();
        {
            let mut s = lock(&self.state);
            let name = s.filename.clone();
            settle_finished(&self.ctx, &mut s, &part, &name)?;
        }
        self.set(Status::Completed, None);
        Ok(())
    }

    async fn run_one(
        self: Arc<Self>,
        first: u64,
        range_ok: bool,
        file: Arc<File>,
        cancel: CancellationToken,
        running: Arc<std::sync::Mutex<HashSet<u64>>>,
    ) -> Result<()> {
        self.acquire(&cancel).await?;
        let result = async {
            let mut current = Some(first);
            while let Some(start) = current {
                lock(&running).insert(start);
                let res = if range_ok {
                    self.run_range_segment(start, &file, &cancel).await
                } else {
                    self.run_stream(start, &file, &cancel).await
                };
                lock(&running).remove(&start);
                res?;
                current = if range_ok && !cancel.is_cancelled() {
                    self.steal(&running)
                } else {
                    None
                };
            }
            Ok(())
        }
        .await;
        self.release();
        result
    }

    /// Splits the running segment with the most bytes left: it keeps the first half and a new segment takes the
    /// second. Done under the lock, so the victim's position cannot move in between.
    fn steal(&self, running: &std::sync::Mutex<HashSet<u64>>) -> Option<u64> {
        let running = lock(running);
        let mut s = lock(&self.state);
        let segs = s.segments.as_mut()?;
        let mut victim: Option<usize> = None;
        for (i, g) in segs.iter().enumerate() {
            if !running.contains(&g.start) {
                continue;
            }
            let left = (g.end + 1).saturating_sub(g.pos);
            if left >= MIN_STEAL && victim.is_none_or(|v| left > segs[v].end + 1 - segs[v].pos) {
                victim = Some(i);
            }
        }
        let v = victim?;
        let g = segs[v];
        let mid = g.pos + (g.end - g.pos).div_ceil(2);
        segs[v].end = mid - 1;
        segs.insert(
            v + 1,
            Segment {
                start: mid,
                end: g.end,
                pos: mid,
            },
        );
        drop(s);
        self.emit();
        Some(mid)
    }

    fn seg(&self, start: u64) -> Option<Segment> {
        lock(&self.state)
            .segments
            .as_ref()?
            .iter()
            .find(|g| g.start == start)
            .copied()
    }

    async fn take(&self, bytes: usize, cancel: &CancellationToken) -> Result<()> {
        self.limiter.take(bytes, cancel).await?;
        self.own.take(bytes, cancel).await
    }

    fn limited(&self) -> bool {
        self.limiter.rate() > 0.0 || self.own.rate() > 0.0
    }

    async fn acquire(&self, cancel: &CancellationToken) -> Result<()> {
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if cancel.is_cancelled() {
                return Err(Fail::Cancelled);
            }
            {
                let mut c = lock(&self.conc);
                if c.workers < c.allowed {
                    c.workers += 1;
                    return Ok(());
                }
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = cancel.cancelled() => return Err(Fail::Cancelled),
            }
        }
    }

    fn release(&self) {
        {
            let mut c = lock(&self.conc);
            c.workers = c.workers.saturating_sub(1);
        }
        self.wake.notify_waiters();
    }

    /// The server answered 429 or 503: one connection fewer (never below one) until things calm down.
    fn slow_down(&self) {
        let adaptive = self.ctx.settings().adaptive;
        let mut c = lock(&self.conc);
        c.last_throttle = Instant::now();
        if adaptive {
            c.allowed = c.allowed.min(c.workers).saturating_sub(1).max(1);
        }
    }

    /// After a quiet spell one connection comes back, up to the configured number.
    fn speed_up(&self) {
        let connections = lock(&self.state).connections as usize;
        let mut woke = false;
        {
            let mut c = lock(&self.conc);
            if c.allowed < connections && c.last_throttle.elapsed() > Duration::from_secs(8) {
                c.allowed += 1;
                c.last_throttle = Instant::now();
                woke = true;
            }
        }
        if woke {
            self.wake.notify_waiters();
        }
    }

    /// Writes what has been gathered at the segment's position. `trim`: keep to the segment's end (a ranged
    /// segment may lose its tail to another connection while bytes wait in the buffer).
    async fn flush(
        &self,
        start: u64,
        file: &Arc<File>,
        buf: &mut Vec<u8>,
        trim: bool,
    ) -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        let Some(seg) = self.seg(start) else {
            buf.clear();
            return Ok(());
        };
        let n = if trim {
            (buf.len() as u64).min((seg.end + 1).saturating_sub(seg.pos)) as usize
        } else {
            buf.len()
        };
        if n > 0 {
            let data = std::mem::take(buf);
            let f = file.clone();
            let at = seg.pos;
            let (data, res) = tokio::task::spawn_blocking(move || {
                let r = write_all_at(&f, &data[..n], at);
                (data, r)
            })
            .await
            .map_err(|e| TaskError::new(e.to_string()))?;
            *buf = data;
            res?;
            let mut s = lock(&self.state);
            if let Some(g) = s
                .segments
                .as_mut()
                .and_then(|v| v.iter_mut().find(|g| g.start == start))
            {
                // `end` may have moved during the write
                let kept = if trim {
                    (n as u64).min((g.end + 1).saturating_sub(at))
                } else {
                    n as u64
                };
                g.pos = at + kept;
                drop(s);
                lock(&self.samples).push_back((Instant::now(), kept));
            }
        }
        buf.clear();
        Ok(())
    }

    async fn run_range_segment(
        &self,
        start: u64,
        file: &Arc<File>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut attempt = 0u32;
        loop {
            let Some(seg) = self.seg(start) else {
                return Ok(());
            };
            if seg.pos > seg.end {
                return Ok(());
            }
            let before = seg.pos;
            let err = match self.range_attempt(start, file, cancel).await {
                Ok(()) => continue,
                Err(Fail::Cancelled) => return Err(Fail::Cancelled),
                Err(Fail::Error(e)) => e,
            };
            if err.fatal {
                return Err(Fail::Error(err));
            }
            if self.seg(start).is_some_and(|g| g.pos > before) {
                attempt = 0;
            }
            attempt += 1;
            if attempt > MAX_RETRIES {
                return Err(Fail::Error(err));
            }
            let backoff = Duration::from_millis((500u64 << attempt).min(8000));
            let wait = backoff.max(Duration::from_secs(err.retry_after_secs));
            if err.throttled {
                // Give the slot back while waiting: with fewer connections allowed, others may go first.
                self.slow_down();
                self.release();
                let waited = async {
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => Ok(()),
                        _ = cancel.cancelled() => Err(Fail::Cancelled),
                    }?;
                    self.acquire(cancel).await
                }
                .await;
                if let Err(e) = waited {
                    lock(&self.conc).workers += 1; // the worker's own release balances this
                    return Err(e);
                }
            } else {
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = cancel.cancelled() => return Err(Fail::Cancelled),
                }
            }
        }
    }

    async fn range_attempt(
        &self,
        start: u64,
        file: &Arc<File>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let (url, mut headers, seg) = {
            let s = lock(&self.state);
            let url = s.final_url.clone().unwrap_or_else(|| s.url.clone());
            let seg = s
                .segments
                .as_ref()
                .and_then(|v| v.iter().find(|g| g.start == start))
                .copied()
                .ok_or_else(|| TaskError::new("Segment introuvable"))?;
            (url.clone(), replay_headers(&self.ctx, &s, &url, true), seg)
        };
        set_header(
            &mut headers,
            "Range",
            &format!("bytes={}-{}", seg.pos, seg.end),
        );
        let mut res = self.ctx.net().get(&url, &headers, cancel).await?;
        let status = res.status().as_u16();
        if status == 200 {
            drop(res);
            return Err(Fail::Error(TaskError {
                message: "Le fichier distant a changé".into(),
                fatal: true,
                changed: true,
                ..TaskError::default()
            }));
        }
        if status != 206 {
            if status >= 500 || status == 429 {
                let retry_after = res
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .unwrap_or(0);
                return Err(Fail::Error(TaskError {
                    message: format!("HTTP {status}"),
                    throttled: status == 429 || status == 503,
                    retry_after_secs: retry_after.min(30),
                    ..TaskError::default()
                }));
            }
            return Err(Fail::Error(TaskError::http(status)));
        }
        let mut buf: Vec<u8> = Vec::with_capacity(BLOCK + 64 * 1024);
        let mut last_flush = Instant::now();
        let outcome: Result<()> = loop {
            let wait = if buf.is_empty() {
                None
            } else {
                Some(FLUSH_EVERY.saturating_sub(last_flush.elapsed()))
            };
            let got = tokio::select! {
                biased;
                _ = cancel.cancelled() => break Err(Fail::Cancelled),
                r = async {
                    match wait {
                        Some(w) => tokio::time::timeout(w, res.chunk()).await.ok(),
                        None => Some(res.chunk().await),
                    }
                } => r,
            };
            let bytes = match got {
                None => {
                    // nothing new for a moment: write what has arrived
                    if let Err(e) = self.flush(start, file, &mut buf, true).await {
                        break Err(e);
                    }
                    last_flush = Instant::now();
                    continue;
                }
                Some(Err(e)) => break Err(Fail::Error(TaskError::from_reqwest(&e))),
                Some(Ok(None)) => break Ok(()),
                Some(Ok(Some(b))) => b,
            };
            // seg.end can shrink while we read: another connection took the tail of this segment.
            let Some(seg) = self.seg(start) else {
                break Ok(());
            };
            let room = (seg.end + 1).saturating_sub(seg.pos + buf.len() as u64) as usize;
            if room == 0 {
                break Ok(());
            }
            let take = bytes.len().min(room);
            if self.limited() && !buf.is_empty() {
                if let Err(e) = self.flush(start, file, &mut buf, true).await {
                    break Err(e);
                }
                last_flush = Instant::now();
            }
            if let Err(e) = self.take(take, cancel).await {
                break Err(e);
            }
            buf.extend_from_slice(&bytes[..take]);
            if buf.len() >= BLOCK || last_flush.elapsed() >= FLUSH_EVERY {
                if let Err(e) = self.flush(start, file, &mut buf, true).await {
                    break Err(e);
                }
                last_flush = Instant::now();
            }
            if self.seg(start).is_none_or(|g| g.pos > g.end) {
                break Ok(());
            }
        };
        drop(res);
        // Whatever the reason we stopped, keep what has arrived: a resume starts after it.
        let flushed = self.flush(start, file, &mut buf, true).await;
        outcome?;
        flushed?;
        if self.seg(start).is_some_and(|g| g.pos <= g.end) {
            return Err(Fail::Error(TaskError::new("Flux interrompu")));
        }
        Ok(())
    }

    /// Server without range support: one sequential stream, restarted from zero on failure.
    async fn run_stream(
        &self,
        start: u64,
        file: &Arc<File>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut attempt = 0u32;
        loop {
            if let Some(g) = lock(&self.state)
                .segments
                .as_mut()
                .and_then(|v| v.iter_mut().find(|g| g.start == start))
            {
                g.pos = 0;
            }
            self.clear_samples();
            let err = match self.stream_attempt(start, file, cancel).await {
                Ok(()) => return Ok(()),
                Err(Fail::Cancelled) => return Err(Fail::Cancelled),
                Err(Fail::Error(e)) => e,
            };
            if err.fatal {
                return Err(Fail::Error(err));
            }
            attempt += 1;
            if attempt > MAX_RETRIES {
                return Err(Fail::Error(err));
            }
            let f = file.clone();
            tokio::task::spawn_blocking(move || f.set_len(0))
                .await
                .map_err(|e| TaskError::new(e.to_string()))??;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis((500u64 << attempt).min(8000))) => {}
                _ = cancel.cancelled() => return Err(Fail::Cancelled),
            }
        }
    }

    async fn stream_attempt(
        &self,
        start: u64,
        file: &Arc<File>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let (url, headers) = {
            let s = lock(&self.state);
            let url = s.final_url.clone().unwrap_or_else(|| s.url.clone());
            (url.clone(), replay_headers(&self.ctx, &s, &url, false))
        };
        let mut res = self.ctx.net().get(&url, &headers, cancel).await?;
        let status = res.status().as_u16();
        if status >= 400 {
            if status >= 500 || status == 429 {
                return Err(Fail::Error(TaskError::new(format!("HTTP {status}"))));
            }
            return Err(Fail::Error(TaskError::http(status)));
        }
        let mut buf: Vec<u8> = Vec::with_capacity(BLOCK + 64 * 1024);
        let mut last_flush = Instant::now();
        let outcome: Result<()> = loop {
            let wait = if buf.is_empty() {
                None
            } else {
                Some(FLUSH_EVERY.saturating_sub(last_flush.elapsed()))
            };
            let got = tokio::select! {
                biased;
                _ = cancel.cancelled() => break Err(Fail::Cancelled),
                r = async {
                    match wait {
                        Some(w) => tokio::time::timeout(w, res.chunk()).await.ok(),
                        None => Some(res.chunk().await),
                    }
                } => r,
            };
            let bytes = match got {
                None => {
                    if let Err(e) = self.flush(start, file, &mut buf, false).await {
                        break Err(e);
                    }
                    last_flush = Instant::now();
                    continue;
                }
                Some(Err(e)) => break Err(Fail::Error(TaskError::from_reqwest(&e))),
                Some(Ok(None)) => break Ok(()),
                Some(Ok(Some(b))) => b,
            };
            if self.limited() && !buf.is_empty() {
                if let Err(e) = self.flush(start, file, &mut buf, false).await {
                    break Err(e);
                }
                last_flush = Instant::now();
            }
            if let Err(e) = self.take(bytes.len(), cancel).await {
                break Err(e);
            }
            buf.extend_from_slice(&bytes);
            if buf.len() >= BLOCK || last_flush.elapsed() >= FLUSH_EVERY {
                if let Err(e) = self.flush(start, file, &mut buf, false).await {
                    break Err(e);
                }
                last_flush = Instant::now();
            }
        };
        drop(res);
        let flushed = self.flush(start, file, &mut buf, false).await;
        outcome?;
        flushed?;
        let mut s = lock(&self.state);
        let pos = s
            .segments
            .as_ref()
            .and_then(|v| v.iter().find(|g| g.start == start))
            .map_or(0, |g| g.pos);
        match s.size {
            None | Some(0) => {
                s.size = Some(pos);
                if let Some(g) = s
                    .segments
                    .as_mut()
                    .and_then(|v| v.iter_mut().find(|g| g.start == start))
                {
                    g.end = pos.saturating_sub(1);
                }
            }
            Some(n) if pos != n => return Err(Fail::Error(TaskError::new("Flux interrompu"))),
            Some(_) => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn split_segments_covers_every_byte_exactly_once() {
        for (size, n) in [(10 * MB + 7, 8u32), (3 * MB, 8), (1, 4), (5 * MB, 1)] {
            let segs = split_segments(size, n);
            assert_eq!(segs[0].start, 0);
            assert_eq!(segs.last().unwrap().end, size - 1);
            for i in 1..segs.len() {
                assert_eq!(segs[i].start, segs[i - 1].end + 1);
            }
        }
        assert_eq!(
            split_segments(3 * MB, 8).len(),
            3,
            "no segment smaller than 1 MiB"
        );
    }
}
