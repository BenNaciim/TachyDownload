//! Helpers shared by the integration tests: a small HTTP/1.1 origin (`test/helpers.js` in the JS suite), temporary
//! folders, random bodies and `wait_for`. Each test file uses a part of it.
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tachy_core::manager::{Manager, ManagerConfig};
use tachy_core::state::{Snapshot, Status};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

pub const MB: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub range: Option<String>,
    pub headers: HashMap<String, String>,
}

/// What the origin answers. `slice` of `body` is sent, `pace` slows it down.
#[derive(Clone)]
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Arc<Vec<u8>>,
    pub from: usize,
    pub to: usize, // exclusive
    pub pace: Option<(usize, Duration)>,
    /// Stop after this many body bytes and close the connection (a dropped stream).
    pub cut_after: Option<usize>,
    /// Wait this long before answering.
    pub delay: Option<Duration>,
}

impl Resp {
    pub fn empty(status: u16) -> Resp {
        Resp {
            status,
            headers: vec![],
            body: Arc::new(vec![]),
            from: 0,
            to: 0,
            pace: None,
            cut_after: None,
            delay: None,
        }
    }
}

pub type Handler = Arc<dyn Fn(&Req) -> Resp + Send + Sync>;

pub struct Server {
    pub url: String,
    pub port: u16,
    pub requests: Arc<Mutex<Vec<Req>>>,
    pub connections: Arc<AtomicUsize>,
    stop: CancellationToken,
}

impl Server {
    pub fn requests(&self) -> Vec<Req> {
        self.requests.lock().unwrap().clone()
    }

    pub fn close(&self) {
        self.stop.cancel();
    }

    pub fn at(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

fn parse_range(h: &str) -> Option<(usize, Option<usize>)> {
    let r = h.strip_prefix("bytes=")?;
    let (a, b) = r.split_once('-')?;
    Some((
        a.parse().ok()?,
        if b.is_empty() {
            None
        } else {
            Some(b.parse().ok()?)
        },
    ))
}

/// A server on a port of its choice (0 = any), answering every request with `handler`.
pub async fn serve(port: u16, handler: Handler) -> Server {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(AtomicUsize::new(0));
    let stop = CancellationToken::new();
    let (reqs, conns, stop2) = (requests.clone(), connections.clone(), stop.clone());
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop2.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((sock, _)) = accepted else { continue };
                    conns.fetch_add(1, Ordering::SeqCst);
                    let (h, r, s) = (handler.clone(), reqs.clone(), stop2.clone());
                    tokio::spawn(async move {
                        tokio::select! { _ = s.cancelled() => {}, _ = connection(sock, h, r) => {} }
                    });
                }
            }
        }
    });
    Server {
        url: format!("http://127.0.0.1:{port}/files/sample.bin"),
        port,
        requests,
        connections,
        stop,
    }
}

async fn connection(mut sock: TcpStream, handler: Handler, requests: Arc<Mutex<Vec<Req>>>) {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        // read one request head
        let head_end = loop {
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
            let mut chunk = [0u8; 4096];
            match sock.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
        buf.drain(..head_end + 4);
        let mut lines = head.split("\r\n");
        let first = lines.next().unwrap_or("");
        let mut parts = first.split(' ');
        let (method, path) = (
            parts.next().unwrap_or("").to_string(),
            parts.next().unwrap_or("/").to_string(),
        );
        let headers: HashMap<String, String> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let req = Req {
            method: method.clone(),
            path,
            range: headers.get("range").cloned(),
            headers,
        };
        requests.lock().unwrap().push(req.clone());
        let resp = handler(&req);
        if let Some(d) = resp.delay {
            tokio::time::sleep(d).await;
        }
        let len = resp.to - resp.from;
        let mut out = format!("HTTP/1.1 {} X\r\n", resp.status);
        let mut has_len = false;
        for (k, v) in &resp.headers {
            has_len |= k.eq_ignore_ascii_case("content-length");
            out.push_str(&format!("{k}: {v}\r\n"));
        }
        if !has_len {
            out.push_str(&format!("Content-Length: {len}\r\n"));
        }
        out.push_str("\r\n");
        if sock.write_all(out.as_bytes()).await.is_err() {
            return;
        }
        if method == "HEAD" {
            continue;
        }
        let data = &resp.body[resp.from..resp.to];
        let limit = resp.cut_after.unwrap_or(data.len()).min(data.len());
        let mut off = 0;
        while off < limit {
            let step = resp.pace.map_or(limit - off, |(s, _)| s).min(limit - off);
            if sock.write_all(&data[off..off + step]).await.is_err() {
                return;
            }
            off += step;
            if let Some((_, every)) = resp.pace {
                tokio::time::sleep(every).await;
            }
        }
        if resp.cut_after.is_some_and(|c| c < data.len()) {
            return; // dropped mid-body
        }
    }
}

/// The answer of a range-capable origin for `body` (or one that ignores `Range` when `ranges` is false).
pub fn ranged(body: &Arc<Vec<u8>>, req: &Req, ranges: bool, disposition: Option<&str>) -> Resp {
    let mut headers = vec![(
        "Content-Type".to_string(),
        "application/octet-stream".to_string(),
    )];
    if let Some(d) = disposition {
        headers.push(("Content-Disposition".into(), d.into()));
    }
    let (mut from, mut to, mut status) = (0usize, body.len(), 200u16);
    if ranges {
        if let Some((a, b)) = req.range.as_deref().and_then(parse_range) {
            from = a.min(body.len());
            to = b.map_or(body.len(), |b| (b + 1).min(body.len()));
            status = 206;
            headers.push((
                "Content-Range".into(),
                format!("bytes {}-{}/{}", from, to.saturating_sub(1), body.len()),
            ));
        }
        headers.push(("Accept-Ranges".into(), "bytes".into()));
    }
    Resp {
        status,
        headers,
        body: body.clone(),
        from,
        to,
        pace: None,
        cut_after: None,
        delay: None,
    }
}

#[derive(Default, Clone)]
pub struct Opts {
    pub no_ranges: bool,
    pub bytes_per_sec: usize,
    pub disposition: Option<String>,
}

/// `startServer(body, {ranges, bytesPerSec, disposition})` of the JS helpers.
pub async fn start_server(body: Vec<u8>, opts: Opts) -> Server {
    let body = Arc::new(body);
    serve(
        0,
        Arc::new(move |req| {
            let mut r = ranged(&body, req, !opts.no_ranges, opts.disposition.as_deref());
            if opts.bytes_per_sec > 0 {
                r.pace = Some(((opts.bytes_per_sec / 20).max(1), Duration::from_millis(50)));
            }
            r
        }),
    )
    .await
}

pub fn tmp_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("dm-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Pseudo-random bytes (a different stream every call).
pub fn random_body(n: usize) -> Vec<u8> {
    let mut x = uuid::Uuid::new_v4().as_u128() as u64 | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

pub async fn wait_for(what: &str, ms: u64, mut f: impl FnMut() -> bool) {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(ms) {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("wait_for: délai dépassé ({what})");
}

pub fn make_manager(dir: &std::path::Path, settings: serde_json::Value) -> Manager {
    let mut s = settings;
    s["dir"] = serde_json::json!(dir.to_string_lossy());
    let m = Manager::new(ManagerConfig {
        settings: s,
        tick_ms: 50,
        ..ManagerConfig::default()
    });
    m.start();
    m
}

pub fn snap(m: &Manager, id: &str) -> Snapshot {
    m.list()
        .into_iter()
        .find(|t| t.state.id == id)
        .expect("task in the list")
}

pub fn is_finished(m: &Manager, id: &str) -> bool {
    matches!(snap(m, id).state.status, Status::Completed | Status::Error)
}

pub async fn finished(m: &Manager, id: &str) {
    wait_for("finished", 30000, || is_finished(m, id)).await;
}

pub fn address(s: &Server) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], s.port))
}
