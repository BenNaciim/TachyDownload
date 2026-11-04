//! Ports of `test/server.test.js` and the API part of `test/refresh.test.js`: the extension's loopback API.
mod common;
use common::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tachy_core::manager::{AddOpts, Manager};
use tachy_core::server::{AddRequest, ExtensionServer, OnRequest, ServerConfig, listen};
use tachy_core::state::Status;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOKEN: &str = "secret-token-for-tests";
const EXT: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

async fn api(m: &Manager, token: &str, on_request: Option<OnRequest>) -> ExtensionServer {
    let token = token.to_string();
    // Port 0 = any free port, so a pooled connection is never reused across tests.
    listen(
        ServerConfig {
            manager: m.clone(),
            token: Arc::new(move || token.clone()),
            on_added: None,
            on_request,
        },
        0,
        1,
    )
    .await
    .unwrap()
}

async fn post(base: &str, path: &str, headers: &[(&str, &str)], body: &str) -> reqwest::Response {
    let mut r = client()
        .post(format!("{base}{path}"))
        .body(body.to_string());
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    r.send().await.unwrap()
}

async fn body_json(r: reqwest::Response) -> Value {
    serde_json::from_slice(&r.bytes().await.unwrap()).unwrap()
}

fn base(srv: &ExtensionServer) -> String {
    format!("http://127.0.0.1:{}", srv.port())
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_requests_without_the_token_from_web_origins_or_with_a_foreign_host() {
    let origin = start_server(random_body(50 * 1024), Opts::default()).await;
    let m = make_manager(&tmp_dir(), json!({}));
    let srv = api(&m, TOKEN, None).await;
    let base = base(&srv);
    let body = json!({ "url": origin.url }).to_string();
    assert_eq!(
        post(&base, "/add", &[("Origin", EXT)], &body)
            .await
            .status(),
        401
    );
    assert_eq!(
        post(
            &base,
            "/add",
            &[("Origin", EXT), ("X-DM-Token", "wrong-token")],
            &body
        )
        .await
        .status(),
        401
    );
    assert_eq!(
        post(
            &base,
            "/add",
            &[("Origin", "https://evil.example"), ("X-DM-Token", TOKEN)],
            &body
        )
        .await
        .status(),
        403
    );
    // a page reaching the port through a name of its own (DNS rebinding)
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", srv.port()))
        .await
        .unwrap();
    s.write_all(b"GET /ping HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 403"), "{out}");
    assert_eq!(m.list().len(), 0);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn accepts_a_valid_add_from_an_extension_and_downloads_the_file() {
    let origin = start_server(random_body(50 * 1024), Opts::default()).await;
    let m = make_manager(&tmp_dir(), json!({}));
    let srv = api(&m, TOKEN, None).await;
    let res = post(
        &base(&srv),
        "/add",
        &[
            ("Origin", EXT),
            ("X-DM-Token", TOKEN),
            ("Content-Type", "application/json"),
        ],
        &json!({ "url": origin.url, "referer": "https://example.com/page" }).to_string(),
    )
    .await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["access-control-allow-origin"], EXT);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();
    wait_for("completed", 15000, || {
        snap(&m, &id).state.status == Status::Completed
    })
    .await;
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ping_answers_without_a_token_and_unknown_routes_are_404() {
    let m = make_manager(&tmp_dir(), json!({}));
    let srv = api(&m, TOKEN, None).await;
    let base = base(&srv);
    let ping = client().get(format!("{base}/ping")).send().await.unwrap();
    assert_eq!(ping.status(), 200);
    assert_eq!(
        body_json(ping).await,
        json!({ "ok": true, "app": "tachy-download" })
    );
    let nope = client()
        .get(format!("{base}/nope"))
        .header("X-DM-Token", TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(nope.status(), 404);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_payloads_are_400_and_cors_preflight_is_answered() {
    let m = make_manager(&tmp_dir(), json!({}));
    let srv = api(&m, TOKEN, None).await;
    let base = base(&srv);
    let h = [("Origin", EXT), ("X-DM-Token", TOKEN)];
    assert_eq!(
        post(&base, "/add", &h, r#"{"url":"file:///etc/passwd"}"#)
            .await
            .status(),
        400
    );
    assert_eq!(post(&base, "/add", &h, "nope").await.status(), 400);
    let pre = client()
        .request(reqwest::Method::OPTIONS, format!("{base}/add"))
        .header("Origin", EXT)
        .send()
        .await
        .unwrap();
    assert_eq!(pre.status(), 204);
    assert!(
        pre.headers()["access-control-allow-headers"]
            .to_str()
            .unwrap()
            .contains("X-DM-Token")
    );
    // a body over 64 KiB is refused
    let big = format!(r#"{{"url":"http://x/","pad":"{}"}}"#, "a".repeat(70_000));
    assert_eq!(post(&base, "/add", &h, &big).await.status(), 413);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn on_request_can_take_over_an_add_nothing_is_queued_until_the_user_answers() {
    let origin = start_server(random_body(50 * 1024), Opts::default()).await;
    let m = make_manager(&tmp_dir(), json!({}));
    let seen: Arc<Mutex<Vec<AddRequest>>> = Arc::new(Mutex::new(vec![]));
    let s2 = seen.clone();
    let srv = api(
        &m,
        TOKEN,
        Some(Arc::new(move |r| {
            s2.lock().unwrap().push(r);
            true
        })),
    )
    .await;
    let base = base(&srv);
    let h = [
        ("Origin", EXT),
        ("X-DM-Token", TOKEN),
        ("Content-Type", "application/json"),
    ];
    let res = post(
        &base,
        "/add",
        &h,
        &json!({ "url": origin.url, "size": 376660, "sniffed": true, "kind": "hls" }).to_string(),
    )
    .await;
    assert_eq!(res.status(), 200);
    assert_eq!(body_json(res).await, json!({ "pending": true }));
    assert_eq!(m.list().len(), 0);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].size, 376660);
        assert_eq!(seen[0].opts.kind.as_deref(), Some("hls"));
        assert!(seen[0].opts.sniffed);
    }
    assert_eq!(
        post(&base, "/add", &h, r#"{"url":"file:///etc/passwd"}"#)
            .await
            .status(),
        400
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_listener_moves_to_the_next_port_when_the_preferred_one_is_taken() {
    let m = make_manager(&tmp_dir(), json!({}));
    let first = api(&m, TOKEN, None).await;
    let taken = first.port();
    let second = listen(
        ServerConfig {
            manager: m.clone(),
            token: Arc::new(|| TOKEN.to_string()),
            on_added: None,
            on_request: None,
        },
        taken,
        10,
    )
    .await
    .unwrap();
    assert!(second.port() > taken && second.port() < taken + 10);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn api_refresh_requests_lists_them_and_refresh_swaps_the_link_token_required() {
    let body = random_body(300 * 1024);
    let valid = Arc::new(Mutex::new("two".to_string()));
    let v = valid.clone();
    let body2 = Arc::new(body.clone());
    let origin = serve(
        0,
        Arc::new(move |req| {
            let sig = req
                .path
                .split_once("sig=")
                .map(|(_, s)| s.to_string())
                .unwrap_or_default();
            if sig != *v.lock().unwrap() {
                return Resp::empty(403);
            }
            ranged(&body2, req, true, None)
        }),
    )
    .await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({}));
    let tok = "tok".repeat(16);
    let srv = api(&m, &tok, None).await;
    let base = base(&srv);
    let id = m
        .add(
            &origin.at("/f.bin?sig=one"),
            &AddOpts {
                filename: Some("a.bin".into()),
                referer: Some("http://page.example/x".into()),
                sniffed: true,
                ..AddOpts::default()
            },
        )
        .unwrap()
        .state
        .id;
    wait_for("error", 15000, || {
        snap(&m, &id).state.status == Status::Error
    })
    .await;
    let no_token = client()
        .get(format!("{base}/refresh-requests"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_token.status(), 401);
    let list: Value = body_json(
        client()
            .get(format!("{base}/refresh-requests"))
            .header("X-DM-Token", &tok)
            .send()
            .await
            .unwrap(),
    )
    .await;
    let requests = list["requests"].as_array().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["id"], json!(id));
    assert_eq!(requests[0]["auto"], json!(true));
    assert_eq!(requests[0]["pageUrl"], json!("http://page.example/x"));

    let h = [
        ("X-DM-Token", tok.as_str()),
        ("Content-Type", "application/json"),
    ];
    let bad = post(
        &base,
        "/refresh",
        &h,
        &json!({ "id": "nope", "url": "http://x/" }).to_string(),
    )
    .await;
    assert_eq!(bad.status(), 400);
    let ok = post(
        &base,
        "/refresh",
        &h,
        &json!({ "id": id, "url": origin.at("/f.bin?sig=two"), "cookie": "a=b" }).to_string(),
    )
    .await;
    assert_eq!(ok.status(), 200);
    wait_for("completed", 15000, || {
        snap(&m, &id).state.status == Status::Completed
    })
    .await;
    assert_eq!(std::fs::read(dir.join("a.bin")).unwrap(), body);
    let cookie = tachy_core::ctx::lock(&m.task(&id).unwrap().state)
        .headers
        .clone()
        .unwrap()
        .cookie;
    assert_eq!(
        cookie, "",
        "the new cookie is dropped once the download is finished"
    );
    m.shutdown().await;
}
