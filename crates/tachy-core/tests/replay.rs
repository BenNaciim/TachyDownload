//! The request context a browser sent (Referer, Origin) is replayed on every request of a download, and the page to
//! reopen for a fresh link is kept apart from it (`test/replay.test.js` in the JS suite).
mod common;
use common::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tachy_core::ctx::lock;
use tachy_core::manager::{AddOpts, RefreshOpts};
use tachy_core::server::{ServerConfig, listen};

const PLAYER: &str = "https://player.example/embed/7";
const PLAYER_ORIGIN: &str = "https://player.example";

/// Serves `routes` (path -> body); the requests it saw are on the server (`requests()`).
async fn recording(routes: HashMap<String, Vec<u8>>) -> Server {
    serve(
        0,
        Arc::new(move |req| {
            let path = req.path.split('?').next().unwrap_or("").to_string();
            match routes.get(&path) {
                None => Resp::empty(404),
                Some(body) => {
                    let body = Arc::new(body.clone());
                    ranged(&body, req, false, None)
                }
            }
        }),
    )
    .await
}

fn header<'a>(r: &'a Req, name: &str) -> Option<&'a str> {
    r.headers.get(name).map(String::as_str)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_download_replays_the_referer_and_the_origin_the_browser_sent() {
    let srv = recording(HashMap::from([(
        "/f.bin".to_string(),
        random_body(200 * 1024),
    )]))
    .await;
    let m = make_manager(&tmp_dir(), json!({}));
    let id = m
        .add(
            &srv.at("/f.bin"),
            &AddOpts {
                referer: Some(PLAYER.into()),
                origin: Some(PLAYER_ORIGIN.into()),
                ..AddOpts::default()
            },
        )
        .unwrap()
        .state
        .id;
    finished(&m, &id).await;
    let done = snap(&m, &id);
    assert_eq!(
        done.state.status,
        tachy_core::state::Status::Completed,
        "{:?}",
        done.state.error
    );
    let seen = srv.requests();
    assert!(seen.len() >= 2, "the probe and the transfer");
    for r in &seen {
        assert_eq!(header(r, "referer"), Some(PLAYER));
        assert_eq!(header(r, "origin"), Some(PLAYER_ORIGIN));
    }
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_hls_stream_replays_them_on_the_playlist_and_on_every_segment() {
    let names = ["s0.ts", "s1.ts", "s2.ts"];
    let mut routes = HashMap::new();
    let playlist: String = names.iter().map(|n| format!("#EXTINF:4,\n{n}\n")).collect();
    routes.insert(
        "/index.m3u8".to_string(),
        format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n{playlist}#EXT-X-ENDLIST\n").into_bytes(),
    );
    for n in names {
        routes.insert(format!("/{n}"), random_body(40_000));
    }
    let srv = recording(routes).await;
    let m = make_manager(&tmp_dir(), json!({ "connections": 2 }));
    let id = m
        .add(
            &srv.at("/index.m3u8"),
            &AddOpts {
                referer: Some(PLAYER.into()),
                origin: Some(PLAYER_ORIGIN.into()),
                filename: Some("film.mp4".into()),
                ..AddOpts::default()
            },
        )
        .unwrap()
        .state
        .id;
    finished(&m, &id).await;
    let done = snap(&m, &id);
    assert_eq!(
        done.state.status,
        tachy_core::state::Status::Completed,
        "{:?}",
        done.state.error
    );
    let seen = srv.requests();
    assert_eq!(seen.len(), 4);
    for r in &seen {
        assert_eq!(header(r, "referer"), Some(PLAYER), "{}", r.path);
        assert_eq!(header(r, "origin"), Some(PLAYER_ORIGIN), "{}", r.path);
    }
    m.shutdown().await;
}

fn page_and_headers(m: &tachy_core::manager::Manager, id: &str) -> (String, String, String) {
    let t = m.task(id).unwrap();
    let s = lock(&t.state);
    let h = s.headers.clone().unwrap_or_default();
    (s.page_url.clone(), h.referer, h.origin)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_to_reopen_for_a_fresh_link_is_kept_apart_from_the_referer_that_is_replayed() {
    let m = make_manager(&tmp_dir(), json!({}));
    let a = m
        .add(
            "http://127.0.0.1:9/a.bin",
            &AddOpts {
                referer: Some(PLAYER.into()),
                origin: Some(PLAYER_ORIGIN.into()),
                page: Some("https://site.example/serie/1".into()),
                sniffed: true,
                paused: true,
                ..AddOpts::default()
            },
        )
        .unwrap();
    let (page, referer, origin) = page_and_headers(&m, &a.state.id);
    assert_eq!(page, "https://site.example/serie/1");
    assert_eq!(referer, PLAYER);
    assert_eq!(origin, PLAYER_ORIGIN);
    let b = m
        .add(
            "http://127.0.0.1:9/b.bin",
            &AddOpts {
                referer: Some("https://site.example/p".into()),
                paused: true,
                ..AddOpts::default()
            },
        )
        .unwrap();
    let (page, _, origin) = page_and_headers(&m, &b.state.id);
    assert_eq!(
        page, "https://site.example/p",
        "without a separate page, the Referer is the page, as before"
    );
    assert_eq!(origin, "");
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_link_brings_its_own_origin_and_keeps_the_page() {
    let m = make_manager(&tmp_dir(), json!({}));
    let id = m
        .add(
            "http://127.0.0.1:9/a.bin",
            &AddOpts {
                referer: Some(PLAYER.into()),
                origin: Some(PLAYER_ORIGIN.into()),
                page: Some("https://site.example/serie/1".into()),
                paused: true,
                ..AddOpts::default()
            },
        )
        .unwrap()
        .state
        .id;
    m.refresh(
        &id,
        &RefreshOpts {
            url: "http://127.0.0.1:9/a2.bin".into(),
            referer: Some("https://other.example/e/1".into()),
            origin: Some("https://other.example".into()),
            ..RefreshOpts::default()
        },
    )
    .unwrap();
    let (page, referer, origin) = page_and_headers(&m, &id);
    assert_eq!(referer, "https://other.example/e/1");
    assert_eq!(origin, "https://other.example");
    assert_eq!(page, "https://site.example/serie/1");
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_extension_api_hands_origin_and_page_over_to_the_confirmation_window_and_to_the_manager()
 {
    const TOKEN: &str = "secret-token-for-tests";
    let ext = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";
    let m = make_manager(&tmp_dir(), json!({}));
    let asked = Arc::new(Mutex::new(Vec::new()));
    let ask = Arc::new(Mutex::new(true));
    let (asked2, ask2) = (asked.clone(), ask.clone());
    let srv = listen(
        ServerConfig {
            manager: m.clone(),
            token: Arc::new(|| TOKEN.to_string()),
            on_added: None,
            on_request: Some(Arc::new(move |r| {
                asked2.lock().unwrap().push(r.opts.clone());
                *ask2.lock().unwrap()
            })),
        },
        0,
        1,
    )
    .await
    .unwrap();
    let client = reqwest::Client::new();
    let post = |payload: Value| {
        let (client, port) = (client.clone(), srv.port());
        async move {
            client
                .post(format!("http://127.0.0.1:{port}/add"))
                .header("Origin", ext)
                .header("X-DM-Token", TOKEN)
                .header("Content-Type", "application/json")
                .body(payload.to_string())
                .send()
                .await
                .unwrap()
        }
    };
    let payload = json!({ "url": "http://127.0.0.1:9/v.bin", "referer": PLAYER, "origin": PLAYER_ORIGIN, "page": "https://site.example/serie/1", "sniffed": true });
    assert_eq!(post(payload.clone()).await.status(), 200);
    {
        let asked = asked.lock().unwrap();
        assert_eq!(asked[0].origin.as_deref(), Some(PLAYER_ORIGIN));
        assert_eq!(
            asked[0].page.as_deref(),
            Some("https://site.example/serie/1")
        );
        assert_eq!(asked[0].referer.as_deref(), Some(PLAYER));
    }
    *ask.lock().unwrap() = false; // no confirmation window: straight to the manager
    let body: Value = serde_json::from_slice(&post(payload).await.bytes().await.unwrap()).unwrap();
    let id = body["id"].as_str().unwrap();
    let (page, _, origin) = page_and_headers(&m, id);
    assert_eq!(origin, PLAYER_ORIGIN);
    assert_eq!(page, "https://site.example/serie/1");
    m.shutdown().await;
    srv.close();
}
