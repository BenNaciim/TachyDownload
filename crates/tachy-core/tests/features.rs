//! Ports of `test/tasklimit.test.js`, `cookies.test.js`, `siteauth.test.js`, `refresh.test.js` (engine part) and
//! `redownload.test.js` (files; the HLS case comes with the HLS module).
mod common;
use common::*;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tachy_core::ctx::lock;
use tachy_core::manager::{AddOpts, Manager, ManagerConfig, RefreshOpts};
use tachy_core::state::Status;

fn add(m: &Manager, url: &str, opts: AddOpts) -> String {
    m.add(url, &opts).unwrap().state.id
}

fn named(name: &str) -> AddOpts {
    AddOpts {
        filename: Some(name.into()),
        ..AddOpts::default()
    }
}

// ---- tasklimit -----------------------------------------------------------------------------------------

/// Time one download takes with the given own limit (KB/s, 0 for none) and shared limit.
async fn time_it(own: f64, shared: u64) -> (u128, tachy_core::state::Snapshot) {
    let body = random_body(700 * 1024);
    let srv = start_server(body.clone(), Opts::default()).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "speedLimitKBps": shared, "connections": 2 }));
    let id = add(
        &m,
        &srv.url,
        AddOpts {
            filename: Some("f.bin".into()),
            paused: true,
            ..AddOpts::default()
        },
    );
    m.set_task_limit(&id, own).unwrap();
    let t0 = Instant::now();
    m.resume(&id).unwrap();
    wait_for("completed", 30000, || {
        snap(&m, &id).state.status == Status::Completed
    })
    .await;
    assert_eq!(std::fs::read(dir.join("f.bin")).unwrap(), body);
    let r = (t0.elapsed().as_millis(), snap(&m, &id));
    m.shutdown().await;
    r
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_with_its_own_limit_is_slowed_down_an_unlimited_one_is_not() {
    let (free, _) = time_it(0.0, 0).await;
    let (slow, snap) = time_it(200.0, 0).await; // the first second is free, the other ~500 KB take ~2.5 s
    assert!(slow > 1800, "limited run took {slow} ms");
    assert!(slow > free * 3, "limited {slow} ms against {free} ms");
    assert_eq!(
        snap.state.speed_limit_kbps,
        Some(200),
        "the limit is part of the task and gets saved"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_shared_limit_still_applies_when_it_is_lower_than_the_own_limit() {
    let (ms, _) = time_it(5000.0, 200).await;
    assert!(ms > 1800, "took {ms} ms");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_task_limit_cleans_its_input_and_zero_removes_the_limit() {
    let m = make_manager(&tmp_dir(), json!({}));
    let id = add(
        &m,
        "http://127.0.0.1:9/x.bin",
        AddOpts {
            filename: Some("x.bin".into()),
            paused: true,
            ..AddOpts::default()
        },
    );
    assert_eq!(
        m.set_task_limit(&id, 123.7).unwrap().state.speed_limit_kbps,
        Some(124)
    );
    assert_eq!(
        m.set_task_limit(&id, -5.0).unwrap().state.speed_limit_kbps,
        Some(0)
    );
    assert_eq!(
        m.set_task_limit(&id, f64::NAN)
            .unwrap()
            .state
            .speed_limit_kbps,
        Some(0)
    );
    assert!(
        m.set_task_limit("nope", 10.0)
            .unwrap_err()
            .contains("introuvable")
    );
    m.shutdown().await;
}

// ---- cookies -------------------------------------------------------------------------------------------

const COOKIE: &str = "session=secret-value";

#[tokio::test(flavor = "multi_thread")]
async fn the_cookie_is_used_for_the_download_then_forgotten_once_it_is_finished() {
    let srv = start_server(random_body(512 * 1024), Opts::default()).await;
    let dir = tmp_dir();
    let store = tmp_dir().join("downloads.json");
    let m = Manager::new(ManagerConfig {
        store_path: Some(store.clone()),
        settings: json!({ "dir": dir.to_string_lossy() }),
        tick_ms: 50,
        ..ManagerConfig::default()
    });
    m.start();
    let id = add(
        &m,
        &srv.url,
        AddOpts {
            cookie: Some(COOKIE.into()),
            ..AddOpts::default()
        },
    );
    let kept = serde_json::to_string(&lock(&m.task(&id).unwrap().state).clone()).unwrap();
    assert!(
        kept.contains(COOKIE),
        "kept while the download may need to resume"
    );
    assert!(
        !serde_json::to_string(&m.list()).unwrap().contains(COOKIE),
        "never sent to the window"
    );
    finished(&m, &id).await;
    assert_eq!(snap(&m, &id).state.status, Status::Completed);
    m.save_now();
    assert!(
        !std::fs::read_to_string(&store).unwrap().contains(COOKIE),
        "gone from the file once completed"
    );
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ytdlp_video_never_stores_a_cookie_and_old_finished_tasks_are_cleaned_at_load() {
    let store = tmp_dir().join("downloads.json");
    let task = |id: &str, name: &str, status: &str| {
        json!({ "id": id, "kind": "file", "url": format!("http://127.0.0.1/{name}"), "dir": tmp_dir().to_string_lossy(),
            "filename": name, "status": status, "headers": { "referer": "", "cookie": COOKIE, "userAgent": "" } })
    };
    std::fs::write(
        &store,
        json!({ "settings": { "dir": tmp_dir().to_string_lossy() },
            "tasks": [task("old", "a.bin", "completed"), task("half", "b.bin", "paused")] })
        .to_string(),
    )
    .unwrap();
    let m = Manager::new(ManagerConfig {
        store_path: Some(store),
        settings: json!({ "dir": tmp_dir().to_string_lossy() }),
        tick_ms: 50,
        ..ManagerConfig::default()
    });
    m.load();
    let cookie_of = |id: &str| {
        lock(&m.task(id).unwrap().state)
            .headers
            .clone()
            .unwrap()
            .cookie
    };
    assert_eq!(cookie_of("old"), "", "finished: cleaned");
    assert_eq!(
        cookie_of("half"),
        COOKIE,
        "unfinished: kept so it can resume"
    );
    let id = add(
        &m,
        "https://www.youtube.com/watch?v=abcdefghijk",
        AddOpts {
            kind: Some("ytdlp".into()),
            cookie: Some(COOKIE.into()),
            paused: true,
            ..AddOpts::default()
        },
    );
    assert_eq!(cookie_of(&id), "");
    m.shutdown().await;
}

// ---- site logins ---------------------------------------------------------------------------------------

const BASIC: &str = "Basic bW9pOnNlY3JldA=="; // moi:secret

/// An origin that wants an Authorization header (or a given cookie when `cookie` is set).
async fn private_server(body: Vec<u8>, cookie: Option<&str>) -> Server {
    let body = Arc::new(body);
    let cookie = cookie.map(str::to_string);
    serve(
        0,
        Arc::new(move |req| {
            let ok = match &cookie {
                Some(c) => req.headers.get("cookie") == Some(c),
                None => req.headers.get("authorization").map(String::as_str) == Some(BASIC),
            };
            if !ok {
                let mut r = Resp::empty(401);
                r.headers.push(("WWW-Authenticate".into(), "Basic".into()));
                return r;
            }
            let mut r = ranged(&body, req, true, None);
            if r.status == 200 {
                r.status = 200;
            }
            r
        }),
    )
    .await
}

fn site_auth_for_loopback(
    asked: Arc<Mutex<Vec<String>>>,
    headers: Vec<(String, String)>,
) -> tachy_core::ctx::SiteAuth {
    Arc::new(move |url: &str| {
        asked.lock().unwrap().push(url.to_string());
        url.starts_with("http://127.0.0.1").then(|| headers.clone())
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_site_login_is_sent_with_every_request_to_that_site() {
    let body = random_body(200 * 1024);
    let srv = private_server(body.clone(), None).await;
    let dir = tmp_dir();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let m = Manager::new(ManagerConfig {
        settings: json!({ "dir": dir.to_string_lossy() }),
        tick_ms: 30,
        site_auth: Some(site_auth_for_loopback(
            asked.clone(),
            vec![("Authorization".into(), BASIC.into())],
        )),
        ..ManagerConfig::default()
    });
    m.start();
    let id = add(&m, &srv.url, named("s.bin"));
    finished(&m, &id).await;
    let t = snap(&m, &id);
    assert_eq!(t.state.status, Status::Completed, "{:?}", t.state.error);
    assert_eq!(std::fs::read(dir.join("s.bin")).unwrap(), body);
    assert!(
        srv.requests()
            .iter()
            .all(|r| r.headers.get("authorization").map(String::as_str) == Some(BASIC)),
        "the probe and every segment carried the login"
    );
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .all(|u| u.starts_with("http://127.0.0.1")),
        "the lookup always gets the address being requested"
    );
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_login_the_download_stops_with_a_message_that_says_what_to_do() {
    let srv = private_server(random_body(10 * 1024), None).await;
    let m = make_manager(&tmp_dir(), json!({}));
    let id = add(&m, &srv.url, named("s.bin"));
    wait_for("error", 15000, || {
        snap(&m, &id).state.status == Status::Error
    })
    .await;
    assert!(
        snap(&m, &id)
            .state
            .error
            .unwrap()
            .contains("Sites connectés")
    );
    assert!(
        srv.requests()
            .iter()
            .all(|r| !r.headers.contains_key("authorization")),
        "nothing was sent"
    );
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_site_cookie_works_like_a_login_and_the_one_from_the_browser_wins() {
    let body = random_body(50 * 1024);
    let srv = private_server(body.clone(), Some("session=browser")).await;
    let dir = tmp_dir();
    let m = Manager::new(ManagerConfig {
        settings: json!({ "dir": dir.to_string_lossy() }),
        tick_ms: 30,
        site_auth: Some(Arc::new(|_: &str| {
            Some(vec![("Cookie".to_string(), "session=saved".to_string())])
        })),
        ..ManagerConfig::default()
    });
    m.start();
    let first = add(&m, &srv.url, named("a.bin"));
    wait_for("error", 15000, || {
        snap(&m, &first).state.status == Status::Error
    })
    .await;
    let second = add(
        &m,
        &srv.url,
        AddOpts {
            filename: Some("b.bin".into()),
            cookie: Some("session=browser".into()),
            ..AddOpts::default()
        },
    );
    wait_for("completed", 15000, || {
        snap(&m, &second).state.status == Status::Completed
    })
    .await;
    assert_eq!(std::fs::read(dir.join("b.bin")).unwrap(), body);
    m.shutdown().await;
}

// ---- refresh -------------------------------------------------------------------------------------------

/// Origin whose links only work with the current `?sig=`; slow enough to be cut in the middle.
async fn signed_server(body: Vec<u8>) -> (Server, Arc<Mutex<String>>) {
    let body = Arc::new(body);
    let valid = Arc::new(Mutex::new("one".to_string()));
    let v = valid.clone();
    let srv = serve(
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
            let mut r = ranged(&body, req, true, None);
            r.pace = Some((64 * 1024, Duration::from_millis(25)));
            r
        }),
    )
    .await;
    (srv, valid)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_link_stops_with_needs_refresh_and_a_fresh_link_resumes_intact() {
    let body = random_body(6 * MB);
    let (srv, valid) = signed_server(body.clone()).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 2 }));
    let id = add(
        &m,
        &srv.at("/f.bin?sig=one"),
        AddOpts {
            filename: Some("big.bin".into()),
            referer: Some("http://page.example/watch".into()),
            ..AddOpts::default()
        },
    );
    wait_for("1 MiB", 30000, || snap(&m, &id).downloaded > MB as u64).await;
    *valid.lock().unwrap() = "two".into(); // the link dies mid-transfer; the next request is refused
    m.pause(&id).await.unwrap();
    m.resume(&id).unwrap();
    wait_for("error", 30000, || {
        snap(&m, &id).state.status == Status::Error
    })
    .await;
    let failed = snap(&m, &id);
    assert!(failed.state.needs_refresh);
    assert!(failed.state.error.as_deref().unwrap().contains("403"));
    assert_eq!(failed.state.page_url, "http://page.example/watch");
    assert_eq!(m.refresh_requests()[0].id, id);
    assert!(failed.downloaded > 0, "partial data is kept");

    m.refresh(
        &id,
        &RefreshOpts {
            url: srv.at("/f.bin?sig=two"),
            ..RefreshOpts::default()
        },
    )
    .unwrap();
    assert_eq!(m.refresh_requests().len(), 0);
    finished(&m, &id).await;
    let done = snap(&m, &id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    assert_eq!(std::fs::read(dir.join("big.bin")).unwrap(), body);
    // It resumed: a ranged request on the new link does not start at byte 0.
    let fresh: Vec<_> = srv
        .requests()
        .into_iter()
        .filter(|r| r.path.ends_with("sig=two") && r.range.is_some())
        .collect();
    assert!(
        fresh
            .iter()
            .any(|r| !r.range.as_deref().unwrap().starts_with("bytes=0-")),
        "resumed from saved offsets"
    );
    m.shutdown().await;
}

// ---- redownload ----------------------------------------------------------------------------------------

type Changing = Arc<Mutex<(Arc<Vec<u8>>, u16)>>;

async fn changing_server(body: Vec<u8>) -> (Server, Changing) {
    let state: Changing = Arc::new(Mutex::new((Arc::new(body), 200)));
    let s = state.clone();
    let srv = serve(
        0,
        Arc::new(move |req| {
            let (data, status) = s.lock().unwrap().clone();
            if status != 200 {
                return Resp::empty(status);
            }
            ranged(&data, req, true, None)
        }),
    )
    .await;
    (srv, state)
}

fn sha(b: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.hash(&mut h);
    h.finish()
}

fn listing(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

struct Fixture {
    m: Manager,
    srv: Server,
    state: Changing,
    dir: std::path::PathBuf,
    id: String,
    v1: Vec<u8>,
}

async fn with_file() -> Fixture {
    let v1 = random_body(3_500_000);
    let (srv, state) = changing_server(v1.clone()).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 4 }));
    let id = add(&m, &srv.at("/files/data.bin"), named("data.bin"));
    finished(&m, &id).await;
    assert_eq!(
        snap(&m, &id).state.status,
        Status::Completed,
        "{:?}",
        snap(&m, &id).state.error
    );
    Fixture {
        m,
        srv,
        state,
        dir,
        id,
        v1,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn redownload_replaces_the_finished_file_under_the_same_name_in_the_same_row() {
    let f = with_file().await;
    let v2 = random_body(4_200_000);
    f.state.lock().unwrap().0 = Arc::new(v2.clone());
    let before = snap(&f.m, &f.id).state.completed_at.unwrap();
    f.m.redownload(&f.id).unwrap();
    assert_eq!(f.m.list().len(), 1, "no new row");
    assert_ne!(
        snap(&f.m, &f.id).state.status,
        Status::Completed,
        "it starts again at once"
    );
    finished(&f.m, &f.id).await;
    let done = snap(&f.m, &f.id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    assert_eq!(done.state.id, f.id);
    assert_eq!(done.state.filename, "data.bin");
    assert_eq!(
        sha(&std::fs::read(f.dir.join("data.bin")).unwrap()),
        sha(&v2)
    );
    assert_ne!(sha(&f.v1), sha(&v2));
    assert_eq!(listing(&f.dir), vec!["data.bin"], "no copy, no leftover");
    assert_eq!(done.state.size, Some(v2.len() as u64));
    assert!(done.state.completed_at.unwrap() >= before);
    f.m.shutdown().await;
    drop(f.srv);
}

#[tokio::test(flavor = "multi_thread")]
async fn redownload_keeps_the_old_file_while_the_new_one_fails_and_a_retry_replaces_it() {
    let f = with_file().await;
    f.state.lock().unwrap().1 = 404;
    f.m.redownload(&f.id).unwrap();
    finished(&f.m, &f.id).await;
    assert_eq!(snap(&f.m, &f.id).state.status, Status::Error);
    assert_eq!(
        sha(&std::fs::read(f.dir.join("data.bin")).unwrap()),
        sha(&f.v1),
        "the old file is untouched"
    );

    let v2 = random_body(3_800_000);
    {
        let mut s = f.state.lock().unwrap();
        s.0 = Arc::new(v2.clone());
        s.1 = 200;
    }
    f.m.resume(&f.id).unwrap();
    finished(&f.m, &f.id).await;
    let done = snap(&f.m, &f.id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    assert_eq!(
        sha(&std::fs::read(f.dir.join("data.bin")).unwrap()),
        sha(&v2)
    );
    assert_eq!(listing(&f.dir), vec!["data.bin"]);
    f.m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn redownload_removing_the_row_while_it_is_failing_leaves_the_old_file_alone() {
    let f = with_file().await;
    f.state.lock().unwrap().1 = 404;
    f.m.redownload(&f.id).unwrap();
    finished(&f.m, &f.id).await;
    f.m.remove(&f.id, true).await.unwrap(); // what "Retirer" asks for on an unfinished download
    assert_eq!(
        sha(&std::fs::read(f.dir.join("data.bin")).unwrap()),
        sha(&f.v1)
    );
    f.m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn redownload_of_a_file_that_is_gone_downloads_it_again_under_its_name() {
    let f = with_file().await;
    std::fs::remove_file(f.dir.join("data.bin")).unwrap();
    let v2 = random_body(3_100_000);
    f.state.lock().unwrap().0 = Arc::new(v2.clone());
    f.m.redownload(&f.id).unwrap();
    finished(&f.m, &f.id).await;
    let done = snap(&f.m, &f.id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    assert_eq!(listing(&f.dir), vec!["data.bin"]);
    assert_eq!(
        sha(&std::fs::read(f.dir.join("data.bin")).unwrap()),
        sha(&v2)
    );
    f.m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn redownload_is_refused_at_once_when_the_file_cannot_be_replaced() {
    let f = with_file().await;
    let file = f.dir.join("data.bin");
    let mut perms = std::fs::metadata(&file).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&file, perms).unwrap();
    let err = f.m.redownload(&f.id).unwrap_err();
    assert!(
        err.contains("autre programme") || err.contains("lecture seule"),
        "{err}"
    );
    assert_eq!(
        snap(&f.m, &f.id).state.status,
        Status::Completed,
        "the finished download is left as it was"
    );
    let mut perms = std::fs::metadata(&file).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&file, perms).unwrap();
    f.m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn redownload_only_a_finished_download_can_be_redone() {
    let (srv, state) = changing_server(vec![]).await;
    state.lock().unwrap().1 = 404;
    let m = make_manager(&tmp_dir(), json!({}));
    let id = add(&m, &srv.at("/files/data.bin"), named("x.bin"));
    finished(&m, &id).await;
    assert_eq!(snap(&m, &id).state.status, Status::Error);
    assert!(m.redownload(&id).unwrap_err().contains("terminé"));
    assert!(m.redownload("nope").unwrap_err().contains("introuvable"));
    m.shutdown().await;
}
