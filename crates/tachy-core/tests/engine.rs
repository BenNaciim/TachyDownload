//! Port of `test/engine.test.js`: the manager end to end against a local origin.
mod common;
use common::*;
use serde_json::json;
use std::time::Duration;
use tachy_core::manager::{AddOpts, Manager, ManagerConfig};
use tachy_core::state::Status;

#[tokio::test(flavor = "multi_thread")]
async fn downloads_with_parallel_segments_and_the_bytes_match() {
    let body = random_body(6 * MB + 123);
    let srv = start_server(body.clone(), Opts::default()).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 4 }));
    let id = m.add(&srv.url, &AddOpts::default()).unwrap().state.id;
    finished(&m, &id).await;
    let t = snap(&m, &id);
    assert_eq!(t.state.status, Status::Completed, "{:?}", t.state.error);
    assert_eq!(t.state.filename, "sample.bin");
    assert_eq!(std::fs::read(dir.join("sample.bin")).unwrap(), body);
    let ranged = srv
        .requests()
        .iter()
        .filter(|r| r.range.as_deref().is_some_and(|x| x != "bytes=0-0"))
        .count();
    assert_eq!(ranged, 4, "one ranged request per segment");
    assert!(!dir.join("sample.bin.part").exists());
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn each_range_gets_its_own_tcp_connection() {
    // HTTP/1.1 only: the segments must not share one connection (a design guarantee, 4.4)
    let srv = start_server(random_body(6 * MB), Opts::default()).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 4 }));
    let id = m.add(&srv.url, &AddOpts::default()).unwrap().state.id;
    finished(&m, &id).await;
    assert_eq!(snap(&m, &id).state.status, Status::Completed);
    let c = srv.connections.load(std::sync::atomic::Ordering::SeqCst);
    assert!(c >= 4, "only {c} TCP connections for 4 ranges");
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn content_disposition_names_the_file_and_duplicates_get_a_suffix() {
    let srv = start_server(
        random_body(200 * 1024),
        Opts {
            disposition: Some("attachment; filename=\"report.bin\"".into()),
            ..Opts::default()
        },
    )
    .await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({}));
    let a = m.add(&srv.url, &AddOpts::default()).unwrap().state.id;
    finished(&m, &a).await;
    let b = m.add(&srv.url, &AddOpts::default()).unwrap().state.id;
    finished(&m, &b).await;
    let mut names: Vec<String> = m.list().into_iter().map(|t| t.state.filename).collect();
    names.sort();
    assert_eq!(names, vec!["report (1).bin", "report.bin"]);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_then_resume_continues_from_the_saved_offsets() {
    let body = random_body(4 * MB);
    let srv = start_server(
        body.clone(),
        Opts {
            bytes_per_sec: 2 * MB,
            ..Opts::default()
        },
    )
    .await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 2 }));
    let id = m.add(&srv.url, &AddOpts::default()).unwrap().state.id;
    wait_for("some bytes", 15000, || {
        snap(&m, &id).downloaded > 300 * 1024
    })
    .await;
    m.pause(&id).await.unwrap();
    let paused = snap(&m, &id);
    assert_eq!(paused.state.status, Status::Paused);
    assert!(paused.downloaded > 0 && (paused.downloaded as usize) < body.len());
    let before = paused.downloaded;
    let requests_before = srv.requests().len();
    m.resume(&id).unwrap();
    finished(&m, &id).await;
    let done = snap(&m, &id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    assert_eq!(std::fs::read(dir.join("sample.bin")).unwrap(), body);
    // Only the requests issued after the resume count: they must ask for the missing bytes, not the whole file again.
    let asked: u64 = srv.requests()[requests_before..]
        .iter()
        .map(|r| {
            let (a, b) = r.range.as_deref().unwrap()[6..].split_once('-').unwrap();
            b.parse::<u64>().unwrap() - a.parse::<u64>().unwrap() + 1
        })
        .sum();
    assert!(
        asked <= body.len() as u64 - before,
        "re-requested {asked} bytes, {} were missing",
        body.len() as u64 - before
    );
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn resumes_after_the_manager_restarts_state_persisted_on_disk() {
    let body = random_body(4 * MB);
    let srv = start_server(
        body.clone(),
        Opts {
            bytes_per_sec: 2 * MB,
            ..Opts::default()
        },
    )
    .await;
    let dir = tmp_dir();
    let store = tmp_dir().join("downloads.json");
    let m1 = Manager::new(ManagerConfig {
        store_path: Some(store.clone()),
        settings: json!({ "dir": dir.to_string_lossy(), "connections": 2 }),
        tick_ms: 50,
        ..ManagerConfig::default()
    });
    m1.start();
    let id = m1.add(&srv.url, &AddOpts::default()).unwrap().state.id;
    wait_for("some bytes", 15000, || {
        snap(&m1, &id).downloaded > 300 * 1024
    })
    .await;
    m1.shutdown().await;

    let m2 = Manager::new(ManagerConfig {
        store_path: Some(store),
        tick_ms: 50,
        ..ManagerConfig::default()
    });
    m2.load();
    m2.start();
    assert_eq!(m2.list()[0].state.status, Status::Paused);
    assert!(m2.list()[0].downloaded > 0);
    m2.resume(&id).unwrap();
    finished(&m2, &id).await;
    assert_eq!(
        m2.list()[0].state.status,
        Status::Completed,
        "{:?}",
        m2.list()[0].state.error
    );
    assert_eq!(std::fs::read(dir.join("sample.bin")).unwrap(), body);
    m2.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn falls_back_to_a_single_stream_when_the_server_ignores_range() {
    let body = random_body(2 * MB + 5);
    let srv = start_server(
        body.clone(),
        Opts {
            no_ranges: true,
            ..Opts::default()
        },
    )
    .await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 8 }));
    let id = m.add(&srv.url, &AddOpts::default()).unwrap().state.id;
    finished(&m, &id).await;
    let t = snap(&m, &id);
    assert_eq!(t.state.status, Status::Completed, "{:?}", t.state.error);
    assert!(!t.state.range_ok);
    assert_eq!(t.state.segments.as_ref().unwrap().len(), 1);
    assert_eq!(std::fs::read(dir.join("sample.bin")).unwrap(), body);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn speed_limit_throttles_the_transfer() {
    let srv = start_server(random_body(600 * 1024), Opts::default()).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 2, "speedLimitKBps": 200 }));
    let t0 = std::time::Instant::now();
    let id = m.add(&srv.url, &AddOpts::default()).unwrap().state.id;
    finished(&m, &id).await;
    let secs = t0.elapsed().as_secs_f64();
    assert_eq!(snap(&m, &id).state.status, Status::Completed);
    // 600 KiB at 200 KiB/s with a one-second burst: about two seconds.
    assert!(secs > 1.5, "finished too fast for the limit: {secs}s");
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_respects_max_concurrent() {
    let srv = start_server(
        random_body(MB),
        Opts {
            bytes_per_sec: MB,
            ..Opts::default()
        },
    )
    .await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "maxConcurrent": 2, "connections": 1 }));
    let ids: Vec<String> = (0..4)
        .map(|_| m.add(&srv.url, &AddOpts::default()).unwrap().state.id)
        .collect();
    let mut peak = 0;
    wait_for("all done", 30000, || {
        let list = m.list();
        peak = peak.max(
            list.iter()
                .filter(|t| t.state.status == Status::Downloading)
                .count(),
        );
        list.iter().all(|t| t.state.status == Status::Completed)
    })
    .await;
    assert_eq!(peak, 2);
    let names: std::collections::HashSet<String> =
        m.list().into_iter().map(|t| t.state.filename).collect();
    assert_eq!(names.len(), ids.len());
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_download_waits_for_its_start_time() {
    let srv = start_server(random_body(100 * 1024), Opts::default()).await;
    let m = make_manager(&tmp_dir(), json!({}));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let id = m
        .add(
            &srv.url,
            &AddOpts {
                start_at: Some(now + 700),
                ..AddOpts::default()
            },
        )
        .unwrap()
        .state
        .id;
    assert_eq!(snap(&m, &id).state.status, Status::Scheduled);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(snap(&m, &id).state.status, Status::Scheduled);
    finished(&m, &id).await;
    assert_eq!(snap(&m, &id).state.status, Status::Completed);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_non_http_links_and_reports_connection_refused() {
    let m = make_manager(&tmp_dir(), json!({}));
    assert!(
        m.add("file:///C:/Windows/system.ini", &AddOpts::default())
            .unwrap_err()
            .contains("http")
    );
    assert!(
        m.add("not a url", &AddOpts::default())
            .unwrap_err()
            .contains("invalide")
    );
    // A port that was free a moment ago and now has nobody listening: a real refusal.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let dead = m
        .add(
            &format!("http://127.0.0.1:{port}/nothing"),
            &AddOpts::default(),
        )
        .unwrap()
        .state
        .id;
    finished(&m, &dead).await;
    let failed = snap(&m, &dead);
    assert_eq!(failed.state.status, Status::Error);
    assert_eq!(
        failed.state.error.as_deref(),
        Some("Le serveur refuse la connexion.")
    );
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn add_with_paused_waits_in_the_list_instead_of_starting() {
    let srv = start_server(random_body(50 * 1024), Opts::default()).await;
    let m = make_manager(&tmp_dir(), json!({}));
    let id = m
        .add(
            &srv.url,
            &AddOpts {
                paused: true,
                ..AddOpts::default()
            },
        )
        .unwrap()
        .state
        .id;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(snap(&m, &id).state.status, Status::Paused);
    m.resume(&id).unwrap();
    wait_for("completed", 15000, || {
        snap(&m, &id).state.status == Status::Completed
    })
    .await;
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn finished_downloads_can_be_taken_off_the_list_in_one_go_their_files_stay() {
    let srv = start_server(random_body(40 * 1024), Opts::default()).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "categories": false }));
    let name = |n: &str| AddOpts {
        filename: Some(n.into()),
        ..AddOpts::default()
    };
    let a = m.add(&srv.url, &name("a.bin")).unwrap().state.id;
    let b = m.add(&srv.url, &name("b.bin")).unwrap().state.id;
    finished(&m, &a).await;
    finished(&m, &b).await;
    let c = m
        .add(
            &srv.url,
            &AddOpts {
                paused: true,
                ..name("c.bin")
            },
        )
        .unwrap()
        .state
        .id;
    assert_eq!(m.clear_completed().await, 2);
    let left: Vec<String> = m.list().into_iter().map(|t| t.state.id).collect();
    assert_eq!(left, vec![c], "only the unfinished one stays");
    assert!(dir.join("a.bin").exists() && dir.join("b.bin").exists());
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_shows_at_once_and_resuming_right_after_still_finishes_the_file() {
    let body = random_body(600 * 1024);
    let srv = start_server(
        body.clone(),
        Opts {
            bytes_per_sec: 300 * 1024,
            ..Opts::default()
        },
    )
    .await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({}));
    let id = m.add(&srv.url, &AddOpts::default()).unwrap().state.id;
    wait_for("downloading", 15000, || {
        snap(&m, &id).state.status == Status::Downloading
    })
    .await;
    let pausing = {
        let (m, id) = (m.clone(), id.clone());
        tokio::spawn(async move { m.pause(&id).await })
    };
    wait_for("paused is shown", 2000, || {
        snap(&m, &id).state.status == Status::Paused
    })
    .await;
    m.resume(&id).unwrap();
    pausing.await.unwrap().unwrap();
    wait_for("completed", 30000, || {
        snap(&m, &id).state.status == Status::Completed
    })
    .await;
    assert_eq!(std::fs::read(dir.join("sample.bin")).unwrap(), body);
    m.shutdown().await;
}
