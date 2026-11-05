//! Loopback API used by the browser extension (design 5.3). Three checks guard it, in this order: the Host header
//! (blocks DNS rebinding), the Origin (only extensions) and a secret token the user copies from the app into the
//! extension once. A small hand-written HTTP/1.1 server: the API has four routes and bodies of 64 KiB at most.

use crate::manager::{AddOpts, Manager, RefreshOpts};
use crate::state::Snapshot;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

const MAX_BODY: usize = 64 * 1024;
const MAX_HEAD: usize = 32 * 1024;

/// What an `/add` carries when the app wants to ask the user before anything is queued.
#[derive(Debug, Clone)]
pub struct AddRequest {
    pub url: String,
    pub size: u64,
    pub opts: AddOpts,
}

pub type TokenFn = Arc<dyn Fn() -> String + Send + Sync>;
pub type OnAdded = Arc<dyn Fn(&Snapshot) + Send + Sync>;
/// Returns true when the app takes the request over (the answer is then `{ "pending": true }`).
pub type OnRequest = Arc<dyn Fn(AddRequest) -> bool + Send + Sync>;

pub struct ServerConfig {
    pub manager: Manager,
    pub token: TokenFn,
    pub on_added: Option<OnAdded>,
    pub on_request: Option<OnRequest>,
}

pub struct ExtensionServer {
    port: u16,
    stop: CancellationToken,
}

impl ExtensionServer {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn close(&self) {
        self.stop.cancel();
    }
}

impl Drop for ExtensionServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// Tries `preferred`, then the next ports (`attempts` in all), and serves on the first that works.
pub async fn listen(
    cfg: ServerConfig,
    preferred: u16,
    attempts: u16,
) -> std::io::Result<ExtensionServer> {
    let mut candidate = preferred;
    let listener = loop {
        match TcpListener::bind(("127.0.0.1", candidate)).await {
            Ok(l) => break l,
            Err(e)
                if e.kind() == std::io::ErrorKind::AddrInUse
                    && candidate < preferred.saturating_add(attempts).saturating_sub(1) =>
            {
                candidate += 1;
            }
            Err(e) => return Err(e),
        }
    };
    let port = listener.local_addr()?.port();
    let stop = CancellationToken::new();
    let shared = Arc::new(Shared {
        cfg,
        port: AtomicU16::new(port),
    });
    let stop2 = stop.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop2.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((sock, _)) = accepted else { continue };
                    let (shared, stop) = (shared.clone(), stop2.clone());
                    tokio::spawn(async move {
                        tokio::select! { _ = stop.cancelled() => {}, _ = connection(sock, shared) => {} }
                    });
                }
            }
        }
    });
    Ok(ExtensionServer { port, stop })
}

struct Shared {
    cfg: ServerConfig,
    port: AtomicU16,
}

struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

enum Read {
    Request(Request),
    TooLarge,
    Closed,
}

async fn read_request(sock: &mut TcpStream, buf: &mut Vec<u8>) -> Read {
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEAD {
            return Read::Closed;
        }
        let mut chunk = [0u8; 4096];
        match sock.read(&mut chunk).await {
            Ok(0) | Err(_) => return Read::Closed,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    buf.drain(..head_end + 4);
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let (method, target) = (
        parts.next().unwrap_or("").to_string(),
        parts.next().unwrap_or("/").to_string(),
    );
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return Read::TooLarge;
    }
    while buf.len() < len {
        let mut chunk = [0u8; 8192];
        match sock.read(&mut chunk).await {
            Ok(0) | Err(_) => return Read::Closed,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let body: Vec<u8> = buf.drain(..len).collect();
    Read::Request(Request {
        method,
        target,
        headers,
        body,
    })
}

fn allowed_origin(origin: &str) -> bool {
    // ^(chrome|moz|edge)-extension:\/\/[a-z0-9-]+$ (case-insensitive)
    let lower = origin.to_ascii_lowercase();
    ["chrome", "moz", "edge"].iter().any(|p| {
        lower
            .strip_prefix(&format!("{p}-extension://"))
            .is_some_and(|id| {
                !id.is_empty()
                    && id
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            })
    })
}

fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

struct Reply {
    status: u16,
    body: Option<Value>,
    extra: Vec<(String, String)>,
    close: bool,
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "OK",
    }
}

async fn write_reply(sock: &mut TcpStream, r: &Reply) -> bool {
    let body = r.body.as_ref().map(|b| b.to_string());
    let mut out = format!("HTTP/1.1 {} {}\r\n", r.status, status_text(r.status));
    if body.is_some() {
        out.push_str("Content-Type: application/json\r\nCache-Control: no-store\r\n");
    }
    for (k, v) in &r.extra {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str(&format!(
        "Content-Length: {}\r\n",
        body.as_ref().map_or(0, String::len)
    ));
    if r.close {
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");
    if let Some(b) = body {
        out.push_str(&b);
    }
    sock.write_all(out.as_bytes()).await.is_ok()
}

async fn connection(mut sock: TcpStream, shared: Arc<Shared>) {
    let mut buf = Vec::new();
    loop {
        let (reply, origin, wants_close) = match read_request(&mut sock, &mut buf).await {
            Read::Closed => return,
            Read::TooLarge => (
                Reply {
                    status: 413,
                    body: Some(json!({ "error": "too large" })),
                    extra: vec![],
                    close: true,
                },
                None,
                true,
            ),
            Read::Request(req) => {
                let origin = req.header("origin").map(str::to_string);
                let close = req
                    .header("connection")
                    .is_some_and(|c| c.eq_ignore_ascii_case("close"));
                (handle(&shared, &req).await, origin, close)
            }
        };
        let mut reply = reply;
        if let Some(o) = origin.as_deref()
            && reply.status != 204
            && allowed_origin(o)
        {
            reply
                .extra
                .push(("Access-Control-Allow-Origin".into(), o.into()));
            reply.extra.push(("Vary".into(), "Origin".into()));
        }
        let close = reply.close || wants_close;
        reply.close = close;
        if !write_reply(&mut sock, &reply).await {
            return;
        }
        if close {
            // Let the client finish sending what it started: closing on unread data would reset the connection
            // and the answer (a 413, say) would be lost.
            let _ = sock.shutdown().await;
            let mut sink = [0u8; 16 * 1024];
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while matches!(sock.read(&mut sink).await, Ok(n) if n > 0) {}
            })
            .await;
            return;
        }
    }
}

fn reply(status: u16, body: Value) -> Reply {
    Reply {
        status,
        body: Some(body),
        extra: vec![],
        close: false,
    }
}

/// A string field, cut to 8192 characters; anything else counts as empty.
fn text(v: &Value) -> String {
    v.as_str()
        .map(|s| s.chars().take(8192).collect())
        .unwrap_or_default()
}

fn some(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

async fn handle(shared: &Shared, req: &Request) -> Reply {
    let origin = req.header("origin");
    let port = shared.port.load(Ordering::SeqCst);
    let host = req.header("host").unwrap_or("");
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return reply(403, json!({ "error": "host" }));
    }
    if origin.is_some_and(|o| !allowed_origin(o)) {
        return reply(403, json!({ "error": "origin" }));
    }
    if req.method == "OPTIONS" {
        let mut extra = vec![];
        if let Some(o) = origin {
            extra.push(("Access-Control-Allow-Origin".to_string(), o.to_string()));
            extra.push(("Vary".to_string(), "Origin".to_string()));
        }
        extra.push((
            "Access-Control-Allow-Methods".into(),
            "GET, POST, OPTIONS".into(),
        ));
        extra.push((
            "Access-Control-Allow-Headers".into(),
            "Content-Type, X-DM-Token".into(),
        ));
        extra.push(("Access-Control-Max-Age".into(), "600".into()));
        return Reply {
            status: 204,
            body: None,
            extra,
            close: false,
        };
    }
    let path = req.target.split('?').next().unwrap_or("/");
    if req.method == "GET" && path == "/ping" {
        return reply(200, json!({ "ok": true, "app": "tachy-download" }));
    }
    let token = (shared.cfg.token)();
    if !same_token(req.header("x-dm-token").unwrap_or(""), &token) {
        return reply(401, json!({ "error": "token" }));
    }
    if req.method == "GET" && path == "/refresh-requests" {
        return reply(
            200,
            json!({ "requests": shared.cfg.manager.refresh_requests() }),
        );
    }
    if req.method == "POST" && (path == "/add" || path == "/refresh") {
        return match post(shared, path, &req.body) {
            Ok(v) => reply(200, v),
            Err(e) => reply(400, json!({ "error": e })),
        };
    }
    reply(404, json!({ "error": "not found" }))
}

fn post(shared: &Shared, path: &str, raw: &[u8]) -> Result<Value, String> {
    let body: Value = serde_json::from_slice(raw).map_err(|e| format!("JSON invalide : {e}"))?;
    let Some(obj) = body.as_object() else {
        return Err("Corps invalide".into());
    };
    let field = |k: &str| obj.get(k).map(text).unwrap_or_default();
    let (referer, origin, cookie, user_agent) = (
        field("referer"),
        field("origin"),
        field("cookie"),
        field("userAgent"),
    );
    let manager = &shared.cfg.manager;
    if path == "/refresh" {
        let task = manager.refresh(
            &field("id"),
            &RefreshOpts {
                url: field("url"),
                referer: some(referer),
                origin: some(origin),
                cookie: some(cookie),
                user_agent: some(user_agent),
            },
        )?;
        return Ok(json!({ "id": task.state.id }));
    }
    let kind = match obj.get("kind").and_then(Value::as_str) {
        Some(k @ ("hls" | "ytdlp")) => Some(k.to_string()),
        _ => None,
    };
    let opts = AddOpts {
        kind,
        filename: some(field("filename")),
        referer: some(referer),
        origin: some(origin),
        cookie: some(cookie),
        user_agent: some(user_agent),
        page: some(field("page")), // the page the stream was found on, to reopen it when the link expires
        sniffed: obj.get("sniffed") == Some(&Value::Bool(true)),
        ..AddOpts::default()
    };
    let url = field("url");
    // The app may want to ask the user (destination, now or later) before adding anything.
    if let Some(ask) = &shared.cfg.on_request {
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err("Adresse invalide".into());
        }
        let size = obj
            .get("size")
            .and_then(|v| {
                v.as_f64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .filter(|n| *n > 0.0)
            .map_or(0, |n| n.round() as u64);
        if ask(AddRequest {
            url: url.clone(),
            size,
            opts: opts.clone(),
        }) {
            return Ok(json!({ "pending": true }));
        }
    }
    let task = manager.add(&url, &opts)?;
    if let Some(f) = &shared.cfg.on_added {
        f(&task);
    }
    Ok(json!({ "id": task.state.id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_extension_origins_are_allowed() {
        assert!(allowed_origin(
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop"
        ));
        assert!(allowed_origin("moz-extension://1234-abcd"));
        assert!(allowed_origin("EDGE-EXTENSION://abc"));
        assert!(!allowed_origin("https://evil.example"));
        assert!(!allowed_origin("chrome-extension://"));
        assert!(!allowed_origin("chrome-extension://abc/def"));
        assert!(!allowed_origin("safari-extension://abc"));
    }

    #[test]
    fn tokens_compare_in_full() {
        assert!(same_token("abc", "abc"));
        assert!(!same_token("abc", "abd"));
        assert!(!same_token("abc", "abcd"));
        assert!(!same_token("", "x"));
    }
}
