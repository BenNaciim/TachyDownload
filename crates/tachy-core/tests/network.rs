//! Ports of `test/steal.test.js`, `retry.test.js` and `network.test.js`: work stealing, automatic retries, the
//! proxy, the speed limits in force and a server that pushes back with 429.
mod common;
use common::*;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tachy_core::manager::{AddOpts, Manager, ManagerConfig};
use tachy_core::net::normalize_proxy;
use tachy_core::state::Status;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

fn add(m: &Manager, url: &str, opts: AddOpts) -> String {
    m.add(url, &opts).unwrap().state.id
}

fn named(name: &str) -> AddOpts {
    AddOpts {
        filename: Some(name.into()),
        ..AddOpts::default()
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// ---- steal ---------------------------------------------------------------------------------------------

// Four segments of 4 MiB: a segment still has at least MIN_STEAL (2 MiB) left for its first 2.5 s, so the other
// connections finding it slow cannot miss the window on a fast machine.
const SIZE: usize = 16 * MB;

/// Range server where the first eighth of the file is served slowly and everything after it fast.
async fn slow_head_server(body: Vec<u8>, slow_bytes_per_sec: usize) -> Server {
    let body = Arc::new(body);
    serve(
        0,
        Arc::new(move |req| {
            let mut r = ranged(&body, req, true, None);
            if r.from < body.len() / 8 {
                r.pace = Some((slow_bytes_per_sec / 20, Duration::from_millis(50)));
            }
            r
        }),
    )
    .await
}

async fn steal_setup() -> (Vec<u8>, Server, std::path::PathBuf, Manager) {
    let body = random_body(SIZE);
    let srv = slow_head_server(body.clone(), 800 * 1024).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 4 }));
    (body, srv, dir, m)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_freed_connection_takes_half_of_the_slowest_segment_and_the_file_stays_identical() {
    let (body, srv, dir, m) = steal_setup().await;
    let t0 = Instant::now();
    let id = add(&m, &srv.at("/f.bin"), AddOpts::default());
    finished(&m, &id).await;
    let t = snap(&m, &id);
    assert_eq!(t.state.status, Status::Completed, "{:?}", t.state.error);
    let segs = t.state.segments.clone().unwrap();
    assert!(
        segs.len() > 4,
        "segments were re-split (got {})",
        segs.len()
    );
    // Without re-splitting the slow first segment alone needs 4 MiB / 800 KiB/s = 5 s.
    assert!(
        t0.elapsed() < Duration::from_millis(4200),
        "finished in {:?}",
        t0.elapsed()
    );
    assert!(std::fs::read(dir.join(&t.state.filename)).unwrap() == body);
    // segments tile the file exactly: no gap, no overlap
    let mut segs = segs;
    segs.sort_by_key(|s| s.start);
    assert_eq!(segs[0].start, 0);
    for w in segs.windows(2) {
        assert_eq!(w[1].start, w[0].end + 1);
    }
    assert_eq!(segs.last().unwrap().end, SIZE as u64 - 1);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_after_a_re_split_and_resuming_still_gives_an_identical_file() {
    let (body, srv, dir, m) = steal_setup().await;
    let id = add(&m, &srv.at("/f.bin"), AddOpts::default());
    wait_for("re-split", 20000, || {
        snap(&m, &id).state.segments.is_some_and(|s| s.len() > 4)
    })
    .await;
    m.pause(&id).await.unwrap();
    assert_eq!(snap(&m, &id).state.status, Status::Paused);
    m.resume(&id).unwrap();
    finished(&m, &id).await;
    let t = snap(&m, &id);
    assert_eq!(t.state.status, Status::Completed, "{:?}", t.state.error);
    assert!(std::fs::read(dir.join(&t.state.filename)).unwrap() == body);
    m.shutdown().await;
}

// ---- retry ---------------------------------------------------------------------------------------------

/// Serves `body` on a fixed port, so the same address can be dead first and alive later.
async fn serve_on(port: u16, body: Vec<u8>, status: u16) -> Server {
    let body = Arc::new(body);
    serve(
        port,
        Arc::new(move |req| {
            if status != 200 {
                return Resp::empty(status);
            }
            let mut r = ranged(&body, req, true, None);
            // like the JS server: plain 200 answers with the whole body, ranges are honoured
            if req.method == "HEAD" {
                r.to = r.from;
            }
            r
        }),
    )
    .await
}

fn retry_manager(dir: &std::path::Path, extra: serde_json::Value) -> Manager {
    let mut s = json!({ "dir": dir.to_string_lossy() });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        s[k] = v;
    }
    let m = Manager::new(ManagerConfig {
        settings: s,
        tick_ms: 30,
        retry_delays: vec![0.1, 0.1, 0.1],
        ..ManagerConfig::default()
    });
    m.start();
    m
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_that_lost_its_server_tries_again_by_itself_and_finishes_when_it_is_back() {
    let body = random_body(256 * 1024);
    let port = free_port();
    let dir = tmp_dir();
    let m = retry_manager(&dir, json!({}));
    let id = add(
        &m,
        &format!("http://127.0.0.1:{port}/a.bin"),
        named("a.bin"),
    );
    wait_for("error with a retry planned", 10000, || {
        let s = snap(&m, &id).state;
        s.status == Status::Error && s.retry_at.is_some()
    })
    .await;
    assert_eq!(snap(&m, &id).state.retry_count, Some(1));
    let _srv = serve_on(port, body.clone(), 200).await;
    wait_for("completed", 20000, || {
        snap(&m, &id).state.status == Status::Completed
    })
    .await;
    assert_eq!(std::fs::read(dir.join("a.bin")).unwrap(), body);
    assert_eq!(snap(&m, &id).state.retry_at, None);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn it_stops_trying_once_every_delay_is_used_and_a_manual_retry_starts_the_count_over() {
    let port = free_port();
    let m = retry_manager(&tmp_dir(), json!({}));
    let id = add(
        &m,
        &format!("http://127.0.0.1:{port}/b.bin"),
        named("b.bin"),
    );
    wait_for("three retries used", 15000, || {
        let s = snap(&m, &id).state;
        s.retry_count == Some(3) && s.status == Status::Error && s.retry_at.is_none()
    })
    .await;
    m.resume(&id).unwrap();
    assert_eq!(snap(&m, &id).state.retry_count, Some(0));
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_from_the_server_is_not_retried() {
    let port = free_port();
    let _srv = serve_on(port, vec![], 404).await;
    let m = retry_manager(&tmp_dir(), json!({}));
    let id = add(
        &m,
        &format!("http://127.0.0.1:{port}/c.bin"),
        named("c.bin"),
    );
    wait_for("error", 10000, || {
        snap(&m, &id).state.status == Status::Error
    })
    .await;
    let s = snap(&m, &id).state;
    assert_eq!(s.retryable, Some(false));
    assert!(s.retry_at.is_none());
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_setting_off_nothing_is_retried() {
    let port = free_port();
    let m = retry_manager(&tmp_dir(), json!({ "autoRetry": false }));
    let id = add(
        &m,
        &format!("http://127.0.0.1:{port}/d.bin"),
        named("d.bin"),
    );
    wait_for("error", 10000, || {
        snap(&m, &id).state.status == Status::Error
    })
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let s = snap(&m, &id).state;
    assert!(s.retry_at.is_none());
    assert_eq!(s.status, Status::Error);
    m.shutdown().await;
}

// ---- proxy and limits ----------------------------------------------------------------------------------

#[test]
fn a_proxy_address_is_cleaned_up_and_an_unusable_one_is_refused() {
    assert_eq!(normalize_proxy("").unwrap(), "");
    assert_eq!(normalize_proxy("  ").unwrap(), "");
    assert_eq!(
        normalize_proxy("proxy.local:3128").unwrap(),
        "http://proxy.local:3128"
    );
    assert_eq!(
        normalize_proxy("http://user:pw@10.0.0.1:8080/").unwrap(),
        "http://user:pw@10.0.0.1:8080"
    );
    assert!(
        normalize_proxy("socks5://127.0.0.1:1080")
            .unwrap_err()
            .contains("http ou https")
    );
    assert!(normalize_proxy("http://").unwrap_err().contains("invalide"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_limit_in_force_is_the_tighter_of_the_fixed_limit_and_the_time_of_day_limit() {
    let m = Manager::new(ManagerConfig {
        settings: json!({ "dir": tmp_dir().to_string_lossy(), "speedLimitKBps": 0,
            "speedSchedule": { "enabled": true, "from": "08:00", "to": "18:00", "limitKBps": 200 } }),
        ..ManagerConfig::default()
    });
    let at = |h: u32| h * 60;
    assert_eq!(m.effective_limit(at(10)), 200, "inside the window");
    assert_eq!(
        m.effective_limit(at(20)),
        0,
        "outside the window: no limit at all"
    );
    m.set_settings(&json!({ "speedLimitKBps": 100 })).unwrap();
    assert_eq!(m.effective_limit(at(10)), 100, "the fixed limit is lower");
    assert_eq!(m.effective_limit(at(20)), 100);
    m.set_settings(&json!({ "speedLimitKBps": 500 })).unwrap();
    assert_eq!(m.effective_limit(at(10)), 200, "the window limit is lower");
    m.set_settings(
        &json!({ "speedSchedule": { "enabled": false, "from": "08:00", "to": "18:00", "limitKBps": 200 } }),
    )
    .unwrap();
    assert_eq!(m.effective_limit(at(10)), 500, "schedule off");
    // a window that crosses midnight
    m.set_settings(&json!({ "speedLimitKBps": 0,
        "speedSchedule": { "enabled": true, "from": "22:00", "to": "06:00", "limitKBps": 50 } }))
        .unwrap();
    assert_eq!(m.effective_limit(at(23)), 50);
    assert_eq!(m.effective_limit(at(3)), 50);
    assert_eq!(m.effective_limit(at(12)), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_shared_bucket_follows_the_limit_in_force() {
    let m = Manager::new(ManagerConfig {
        settings: json!({ "dir": tmp_dir().to_string_lossy(), "speedLimitKBps": 300 }),
        ..ManagerConfig::default()
    });
    m.start();
    m.tick();
    assert_eq!(m.limiter().rate(), 300.0 * 1024.0);
    m.set_settings(&json!({ "speedLimitKBps": 0 })).unwrap();
    m.tick();
    assert_eq!(m.limiter().rate(), 0.0);
    m.shutdown().await;
}

/// A tiny HTTP proxy: plain requests are forwarded (the head goes on as it came), CONNECT is tunnelled. Counts
/// what went through.
async fn start_proxy() -> (String, Arc<AtomicUsize>, CancellationToken) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(AtomicUsize::new(0));
    let stop = CancellationToken::new();
    let (seen2, stop2) = (seen.clone(), stop.clone());
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop2.cancelled() => break,
                Ok((mut client, _)) = listener.accept() => {
                    let seen = seen2.clone();
                    tokio::spawn(async move {
                        let mut buf = Vec::new();
                        let mut chunk = [0u8; 4096];
                        let end = loop {
                            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") { break i + 4; }
                            match client.read(&mut chunk).await { Ok(0) | Err(_) => return, Ok(n) => buf.extend_from_slice(&chunk[..n]) }
                        };
                        seen.fetch_add(1, Ordering::SeqCst);
                        let head = String::from_utf8_lossy(&buf[..end]).into_owned();
                        let first = head.lines().next().unwrap_or("").to_string();
                        let mut parts = first.split(' ');
                        let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                        if method == "CONNECT" {
                            let Ok(mut up) = TcpStream::connect(target).await else { return };
                            let _ = client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await;
                            let _ = client.write_all(&[]).await;
                            let _ = tokio::io::copy_bidirectional(&mut client, &mut up).await;
                        } else {
                            let url = url::Url::parse(target).unwrap();
                            let addr = format!("{}:{}", url.host_str().unwrap(), url.port().unwrap_or(80));
                            let Ok(mut up) = TcpStream::connect(addr).await else { return };
                            let path = match url.query() { Some(q) => format!("{}?{q}", url.path()), None => url.path().to_string() };
                            let rest = &head[first.len()..];
                            let _ = up.write_all(format!("{method} {path} HTTP/1.1{rest}").as_bytes()).await;
                            let _ = up.write_all(&buf[end..]).await;
                            let _ = tokio::io::copy_bidirectional(&mut client, &mut up).await;
                        }
                    });
                }
            }
        }
    });
    (format!("127.0.0.1:{port}"), seen, stop)
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_go_through_the_proxy_when_one_is_set() {
    let body = random_body(300 * 1024);
    let srv = start_server(body.clone(), Opts::default()).await;
    let (proxy, seen, stop) = start_proxy().await;
    let dir = tmp_dir();
    let m = Manager::new(ManagerConfig {
        settings: json!({ "dir": dir.to_string_lossy(), "proxy": proxy }),
        tick_ms: 30,
        ..ManagerConfig::default()
    });
    m.start();
    assert_eq!(m.settings().proxy, format!("http://{proxy}"));
    let id = add(&m, &srv.url, named("p.bin"));
    wait_for("completed", 30000, || {
        snap(&m, &id).state.status == Status::Completed
    })
    .await;
    assert_eq!(std::fs::read(dir.join("p.bin")).unwrap(), body);
    assert!(
        seen.load(Ordering::SeqCst) > 0,
        "the proxy carried the traffic"
    );

    // without the proxy the traffic no longer goes through it
    m.set_settings(&json!({ "proxy": "" })).unwrap();
    let before = seen.load(Ordering::SeqCst);
    let second = add(&m, &srv.url, named("q.bin"));
    wait_for("completed", 30000, || {
        snap(&m, &second).state.status == Status::Completed
    })
    .await;
    assert_eq!(seen.load(Ordering::SeqCst), before);
    m.shutdown().await;
    stop.cancel();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_proxy_is_refused_when_saving_settings_and_the_old_one_stays() {
    let m = Manager::new(ManagerConfig {
        settings: json!({ "dir": tmp_dir().to_string_lossy(), "proxy": "proxy.local:3128" }),
        ..ManagerConfig::default()
    });
    assert!(
        m.set_settings(&json!({ "proxy": "ftp://x" }))
            .unwrap_err()
            .contains("http ou https")
    );
    assert_eq!(m.settings().proxy, "http://proxy.local:3128");
}

// ---- a server that pushes back -------------------------------------------------------------------------

struct Picky {
    port: u16,
    refused: Arc<AtomicUsize>,
    stop: CancellationToken,
}

impl Drop for Picky {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

struct Inflight(Arc<AtomicUsize>);
impl Drop for Inflight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Origin that answers 429 as soon as more than `max` requests are in flight at once.
async fn start_picky(body: Vec<u8>, max: usize) -> Picky {
    let body = Arc::new(body);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (inflight, refused) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let stop = CancellationToken::new();
    let (refused2, stop2) = (refused.clone(), stop.clone());
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop2.cancelled() => break,
                Ok((mut sock, _)) = listener.accept() => {
                    let (body, inflight, refused, stop) = (body.clone(), inflight.clone(), refused2.clone(), stop2.clone());
                    tokio::spawn(async move {
                        tokio::select! { _ = stop.cancelled() => {}, _ = async {
                            let mut buf = Vec::new();
                            loop {
                                let end = loop {
                                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") { break i + 4; }
                                    let mut chunk = [0u8; 4096];
                                    match sock.read(&mut chunk).await { Ok(0) | Err(_) => return, Ok(n) => buf.extend_from_slice(&chunk[..n]) }
                                };
                                let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                                buf.drain(..end);
                                let range = head.lines().find_map(|l| l.strip_prefix("range: bytes=")).map(str::trim).and_then(|r| r.split_once('-'))
                                    .map(|(a, b)| (a.parse::<usize>().unwrap_or(0), b.parse::<usize>().ok()));
                                let start = range.map_or(0, |r| r.0);
                                let last = range.and_then(|r| r.1).map_or(body.len() - 1, |e| e.min(body.len() - 1));
                                let is_probe = start == 0 && last == 0;
                                if !is_probe && inflight.load(Ordering::SeqCst) >= max {
                                    refused.fetch_add(1, Ordering::SeqCst);
                                    if sock.write_all(b"HTTP/1.1 429 X\r\nContent-Length: 0\r\n\r\n").await.is_err() { return; }
                                    continue;
                                }
                                inflight.fetch_add(1, Ordering::SeqCst);
                                let _guard = Inflight(inflight.clone());
                                let slice = &body[start..=last];
                                let out = format!("HTTP/1.1 206 X\r\nContent-Type: application/octet-stream\r\nContent-Range: bytes {start}-{last}/{}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n", body.len(), slice.len());
                                if sock.write_all(out.as_bytes()).await.is_err() { return; }
                                let step = slice.len().div_ceil(8).max(1);
                                for part in slice.chunks(step) {
                                    if sock.write_all(part).await.is_err() { return; }
                                    tokio::time::sleep(Duration::from_millis(40)).await;
                                }
                            }
                        } => {} }
                    });
                }
            }
        }
    });
    Picky {
        port,
        refused,
        stop,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_refuses_too_many_connections_ends_up_with_fewer_and_the_file_is_complete() {
    let body = random_body(6 * MB);
    let srv = start_picky(body.clone(), 2).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 6 }));
    let id = add(
        &m,
        &format!("http://127.0.0.1:{}/big.bin", srv.port),
        named("big.bin"),
    );
    wait_for("finished", 50000, || is_finished(&m, &id)).await;
    let t = snap(&m, &id);
    assert_eq!(t.state.status, Status::Completed, "{:?}", t.state.error);
    assert_eq!(std::fs::read(dir.join("big.bin")).unwrap(), body);
    assert!(
        srv.refused.load(Ordering::SeqCst) > 0,
        "the server did push back"
    );
    let allowed = m.task(&id).unwrap().allowed();
    assert!(allowed <= 2, "connections allowed fell to {allowed}");
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_adaptive_setting_off_the_number_of_connections_is_left_alone() {
    let body = random_body(3 * MB);
    let srv = start_picky(body, 2).await;
    let m = make_manager(&tmp_dir(), json!({ "connections": 3, "adaptive": false }));
    let id = add(
        &m,
        &format!("http://127.0.0.1:{}/big.bin", srv.port),
        named("big.bin"),
    );
    wait_for("finished", 50000, || is_finished(&m, &id)).await;
    assert_eq!(m.task(&id).unwrap().allowed(), 3);
    m.shutdown().await;
}

// ---- web page or file ----------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn an_address_that_answers_with_a_web_page_is_told_apart_from_a_file() {
    let srv = serve(
        0,
        Arc::new(|req| {
            let (status, ctype, body): (u16, &str, &[u8]) = match req.path.as_str() {
                "/page" => (200, "text/html; charset=utf-8", b"<html></html>"),
                "/gone" => (404, "text/html", b"no"),
                _ => (200, "video/mp4", b"x"),
            };
            let mut r = Resp::empty(status);
            r.headers.push(("Content-Type".into(), ctype.into()));
            r.body = Arc::new(body.to_vec());
            r.to = body.len();
            r
        }),
    )
    .await;
    let net = tachy_core::net::Net::new("");
    let h = Vec::new();
    use tachy_core::probe::is_web_page;
    assert!(is_web_page(&net, &srv.at("/page"), &h).await);
    assert!(!is_web_page(&net, &srv.at("/clip"), &h).await);
    assert!(!is_web_page(&net, &srv.at("/gone"), &h).await);
    assert!(!is_web_page(&net, "http://127.0.0.1:1/x", &h).await);
}
