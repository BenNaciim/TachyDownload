//! Port of `test/hls.test.js` (the playlist parsing tests are unit tests in `task/hls.rs`) and of the stream case
//! of `test/redownload.test.js`.
mod common;
use aes::Aes128;
use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use common::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tachy_core::manager::{AddOpts, Manager, ManagerConfig};
use tachy_core::state::{Kind, Status};

type Routes = Arc<Mutex<HashMap<String, Vec<u8>>>>;

fn routes(list: Vec<(String, Vec<u8>)>) -> Routes {
    Arc::new(Mutex::new(list.into_iter().collect()))
}

/// Serves `routes` (path -> body); `slow_ms` delays segment answers, `delays` (path -> ms) delays chosen ones.
async fn hls_server(routes: Routes, slow_ms: u64, delays: Vec<(&str, u64)>) -> Server {
    let delays: HashMap<String, u64> = delays
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    serve(
        0,
        Arc::new(move |req| {
            let p = req.path.split('?').next().unwrap().to_string();
            let Some(body) = routes.lock().unwrap().get(&p).cloned() else {
                return Resp::empty(404);
            };
            let wait =
                delays
                    .get(&p)
                    .copied()
                    .unwrap_or(if p.starts_with("/seg") { slow_ms } else { 0 });
            let len = body.len();
            let mut r = Resp::empty(200);
            r.headers
                .push(("Content-Type".into(), "application/octet-stream".into()));
            r.body = Arc::new(body);
            r.to = len;
            r.delay = (wait > 0).then(|| Duration::from_millis(wait));
            r
        }),
    )
    .await
}

fn media(names: &[String], extra: &str) -> Vec<u8> {
    let items: String = names.iter().map(|n| format!("#EXTINF:4,\n{n}\n")).collect();
    format!("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n{extra}{items}#EXT-X-ENDLIST\n")
        .into_bytes()
}

fn seg_names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("seg{i}.ts")).collect()
}

fn add(m: &Manager, url: &str, filename: Option<&str>) -> String {
    m.add(
        url,
        &AddOpts {
            filename: filename.map(str::to_string),
            ..AddOpts::default()
        },
    )
    .unwrap()
    .state
    .id
}

fn hits(srv: &Server) -> Vec<String> {
    srv.requests()
        .iter()
        .map(|r| r.path.split('?').next().unwrap().to_string())
        .collect()
}

fn more(m: &Manager, id: &str, key: &str) -> Value {
    serde_json::to_value(snap(m, id)).unwrap()[key].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn master_playlist_best_variant_segments_joined_in_order() {
    let segs: Vec<Vec<u8>> = (0..9).map(|_| random_body(50_000)).collect();
    let mut list = vec![
        (
            "/master.m3u8".to_string(),
            b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=900\nhigh.m3u8\n".to_vec(),
        ),
        ("/low.m3u8".into(), media(&["seg-low.ts".into()], "")),
        ("/seg-low.ts".into(), b"nope".to_vec()),
        ("/high.m3u8".into(), media(&seg_names(9), "")),
    ];
    for (i, b) in segs.iter().enumerate() {
        list.push((format!("/seg{i}.ts"), b.clone()));
    }
    let srv = hls_server(routes(list), 20, vec![]).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 4 }));
    let t = m
        .add(
            &srv.at("/master.m3u8"),
            &AddOpts {
                filename: Some("film.mp4".into()),
                ..AddOpts::default()
            },
        )
        .unwrap();
    assert_eq!(t.state.kind, Kind::Hls);
    let id = t.state.id;
    finished(&m, &id).await;
    let done = snap(&m, &id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    assert_eq!(done.state.filename, "film.ts");
    assert_eq!(std::fs::read(dir.join("film.ts")).unwrap(), segs.concat());
    assert!(!hits(&srv).contains(&"/seg-low.ts".to_string()));
    assert_eq!(Some(done.downloaded), done.state.size);
    m.shutdown().await;
}

fn encrypt(buf: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Vec<u8> {
    cbc::Encryptor::<Aes128>::new(key.into(), iv.into()).encrypt_padded_vec_mut::<Pkcs7>(buf)
}

#[tokio::test(flavor = "multi_thread")]
async fn aes_128_segments_are_decrypted_explicit_iv_and_sequence_iv() {
    let key: [u8; 16] = random_body(16).try_into().unwrap();
    let plain = [random_body(30_000), random_body(30_001)];
    let mut seq7 = [0u8; 16];
    seq7[12..].copy_from_slice(&7u32.to_be_bytes()); // media sequence 7 -> first segment
    let mut seq8 = [0u8; 16];
    seq8[12..].copy_from_slice(&8u32.to_be_bytes());
    let list = vec![
        ("/k.bin".to_string(), key.to_vec()),
        (
            "/v.m3u8".into(),
            media(
                &seg_names(2),
                "#EXT-X-MEDIA-SEQUENCE:7\n#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\"\n",
            ),
        ),
        ("/seg0.ts".into(), encrypt(&plain[0], &key, &seq7)),
        ("/seg1.ts".into(), encrypt(&plain[1], &key, &seq8)),
    ];
    let srv = hls_server(routes(list), 0, vec![]).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({}));
    let id = add(&m, &srv.at("/v.m3u8"), None);
    finished(&m, &id).await;
    let done = snap(&m, &id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    assert_eq!(
        std::fs::read(dir.join(&done.state.filename)).unwrap(),
        plain.concat()
    );
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn drm_and_live_streams_are_refused_with_a_clear_message() {
    let list = vec![
        (
            "/drm.m3u8".to_string(),
            media(
                &["seg0.ts".into()],
                "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\n",
            ),
        ),
        (
            "/live.m3u8".into(),
            b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nseg0.ts\n".to_vec(),
        ),
        ("/seg0.ts".into(), b"x".to_vec()),
    ];
    let srv = hls_server(routes(list), 0, vec![]).await;
    let m = make_manager(&tmp_dir(), json!({}));
    let a = add(&m, &srv.at("/drm.m3u8"), None);
    let b = add(&m, &srv.at("/live.m3u8"), None);
    finished(&m, &a).await;
    finished(&m, &b).await;
    assert!(snap(&m, &a).state.error.unwrap().contains("DRM"));
    assert!(snap(&m, &b).state.error.unwrap().contains("directs"));
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_then_resume_continues_without_refetching_finished_segments() {
    let segs: Vec<Vec<u8>> = (0..12).map(|_| random_body(20_000)).collect();
    let mut list = vec![("/v.m3u8".to_string(), media(&seg_names(12), ""))];
    for (i, b) in segs.iter().enumerate() {
        list.push((format!("/seg{i}.ts"), b.clone()));
    }
    let srv = hls_server(routes(list), 120, vec![]).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 2 }));
    let id = add(&m, &srv.at("/v.m3u8"), Some("clip"));
    wait_for("3 segments", 15000, || {
        more(&m, &id, "hlsDone").as_u64().unwrap_or(0) >= 3
    })
    .await;
    m.pause(&id).await.unwrap();
    let seg_hits = |s: &Server| hits(s).iter().filter(|h| h.starts_with("/seg")).count();
    let before = seg_hits(&srv);
    assert_eq!(snap(&m, &id).state.status, Status::Paused);
    m.resume(&id).unwrap();
    finished(&m, &id).await;
    let done = snap(&m, &id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    assert_eq!(
        std::fs::read(dir.join(&done.state.filename)).unwrap(),
        segs.concat()
    );
    let total = seg_hits(&srv);
    assert!(
        total - before <= 12 - 3 + 2,
        "refetched too much: {}",
        total - before
    );
    m.shutdown().await;
}

/// 12 small segments whose first one is held back for `hold_ms`; the rest answer at once.
async fn stalled_first_segment(
    hold_ms: u64,
    ahead: Option<u64>,
) -> (Vec<Vec<u8>>, Server, std::path::PathBuf, Manager, String) {
    let segs: Vec<Vec<u8>> = (0..12).map(|_| random_body(20_000)).collect();
    let mut list = vec![("/v.m3u8".to_string(), media(&seg_names(12), ""))];
    for (i, b) in segs.iter().enumerate() {
        list.push((format!("/seg{i}.ts"), b.clone()));
    }
    let srv = hls_server(routes(list), 0, vec![("/seg0.ts", hold_ms)]).await;
    let dir = tmp_dir();
    let m = Manager::new(ManagerConfig {
        settings: json!({ "dir": dir.to_string_lossy(), "connections": 3 }),
        tick_ms: 50,
        hls_ahead_bytes: ahead,
        ..ManagerConfig::default()
    });
    m.start();
    let id = add(&m, &srv.at("/v.m3u8"), Some("clip"));
    (segs, srv, dir, m, id)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_segment_does_not_keep_the_other_connections_from_fetching_ahead() {
    let (segs, srv, dir, m, id) = stalled_first_segment(1500, None).await;
    // With 3 connections a strict queue stops at the third segment until the first one arrives.
    wait_for("seg11 requested", 1200, || {
        hits(&srv).contains(&"/seg11.ts".to_string())
    })
    .await;
    let snap1 = snap(&m, &id);
    assert_eq!(
        snap1.downloaded, 0,
        "nothing can be written before the first segment"
    );
    assert_eq!(more(&m, &id, "hlsTotal"), json!(12));
    assert_eq!(more(&m, &id, "hlsDone"), json!(0));
    let live = more(&m, &id, "hlsLive");
    assert!(
        live.as_array().unwrap().contains(&json!(0))
            && live.as_array().unwrap().contains(&json!(11)),
        "in progress or received: {live}"
    );
    finished(&m, &id).await;
    let done = snap(&m, &id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    assert_eq!(
        std::fs::read(dir.join(&done.state.filename)).unwrap(),
        segs.concat(),
        "segments stay in order"
    );
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn what_is_received_ahead_of_a_stalled_segment_is_capped_then_the_download_finishes() {
    // 20 kB segments and a 50 kB cap: after the first one stalls, only a few more may be taken ahead.
    let (segs, srv, dir, m, id) = stalled_first_segment(1200, Some(50_000)).await;
    wait_for("seg3 requested", 1000, || {
        hits(&srv).contains(&"/seg3.ts".to_string())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await; // time for any over-eager fetch to show up
    assert!(
        !hits(&srv).contains(&"/seg8.ts".to_string()),
        "fetched far beyond the cap: {}",
        hits(&srv).join(" ")
    );
    finished(&m, &id).await;
    let done = snap(&m, &id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    assert_eq!(
        std::fs::read(dir.join(&done.state.filename)).unwrap(),
        segs.concat()
    );
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn redownload_a_video_stream_is_replaced_under_its_name_too() {
    let make = |tag: &str| -> Vec<Vec<u8>> {
        (0..3)
            .map(|i| format!("{tag}-{i}-").repeat(4000).into_bytes())
            .collect()
    };
    let publish = |r: &Routes, list: &[Vec<u8>]| {
        let mut r = r.lock().unwrap();
        for (i, b) in list.iter().enumerate() {
            r.insert(format!("/seg{i}.ts"), b.clone());
        }
        r.insert("/v.m3u8".into(), media(&seg_names(list.len()), ""));
    };
    let r = routes(vec![]);
    publish(&r, &make("old"));
    let srv = hls_server(r.clone(), 0, vec![]).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "convertToMp4": false }));
    let id = add(&m, &srv.at("/v.m3u8"), Some("clip"));
    finished(&m, &id).await;
    assert_eq!(
        snap(&m, &id).state.status,
        Status::Completed,
        "{:?}",
        snap(&m, &id).state.error
    );
    let fresh = make("new");
    publish(&r, &fresh);
    m.redownload(&id).unwrap();
    finished(&m, &id).await;
    let done = snap(&m, &id);
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec![done.state.filename.clone()]);
    assert_eq!(
        std::fs::read(dir.join(&done.state.filename)).unwrap(),
        fresh.concat()
    );
    m.shutdown().await;
}
