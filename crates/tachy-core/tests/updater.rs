//! Port of `test/updater.test.js`: which release is offered, and the installer is only kept when its SHA-256 is the
//! one GitHub published.
mod common;
use common::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering as AtomicOrdering};
use tachy_core::updater::{check_for_update, compare_versions, download_update};

fn installer() -> Vec<u8> {
    "MZ-pretend-installer".repeat(5000).into_bytes()
}

fn sha(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn reply(body: Vec<u8>, status: u16) -> Resp {
    let to = body.len();
    Resp {
        status,
        body: Arc::new(body),
        to,
        ..Resp::empty(status)
    }
}

/// A stand-in for GitHub: `/repos/.../releases/latest` answers `release(port)` (404 when none), `/dl/setup.exe` the
/// installer.
async fn mock_github(release: Option<Box<dyn Fn(u16) -> Value + Send + Sync>>) -> Server {
    let port = Arc::new(AtomicU16::new(0));
    let p = port.clone();
    let handler: Handler = Arc::new(move |req| {
        if req.path.ends_with("/releases/latest") {
            return match &release {
                None => reply(b"{}".to_vec(), 404),
                Some(r) => reply(
                    r(p.load(AtomicOrdering::SeqCst)).to_string().into_bytes(),
                    200,
                ),
            };
        }
        if req.path == "/dl/setup.exe" {
            return reply(installer(), 200);
        }
        reply(vec![], 404)
    });
    let srv = serve(0, handler).await;
    port.store(srv.port, AtomicOrdering::SeqCst);
    srv
}

fn rel(over: Value) -> Option<Box<dyn Fn(u16) -> Value + Send + Sync>> {
    Some(Box::new(move |port| {
        let mut v = json!({
            "tag_name": "v1.2.0",
            "html_url": "https://github.com/x/y/releases/tag/v1.2.0",
            "assets": [
                { "name": "notes.txt", "browser_download_url": format!("http://127.0.0.1:{port}/dl/notes.txt"), "size": 1 },
                { "name": "Tachy-Download-Setup-1.2.0.exe",
                  "browser_download_url": format!("http://127.0.0.1:{port}/dl/setup.exe"),
                  "size": installer().len(),
                  "digest": format!("sha256:{}", sha(&installer())) },
            ],
        });
        if let (Some(o), Some(extra)) = (v.as_object_mut(), over.as_object()) {
            o.extend(extra.clone());
        }
        v
    }))
}

fn api(s: &Server) -> String {
    format!("http://127.0.0.1:{}", s.port)
}

#[test]
fn versions_are_ordered_numerically_ignoring_a_leading_v() {
    assert_eq!(compare_versions("v1.10.0", "1.9.9"), Ordering::Greater);
    assert_eq!(compare_versions("1.0.0", "v1.0.0"), Ordering::Equal);
    assert_eq!(compare_versions("1.0", "1.0.1"), Ordering::Less);
    assert_eq!(compare_versions("2.0.0-beta.1", "2.0.0"), Ordering::Equal);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newer_release_with_an_installer_is_offered_with_its_checksum() {
    let srv = mock_github(rel(json!({}))).await;
    let info = check_for_update("1.0.0", &api(&srv))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.version, "1.2.0");
    assert_eq!(info.sha256.as_deref(), Some(sha(&installer()).as_str()));
    assert!(info.url.ends_with("/dl/setup.exe"));
    assert_eq!(info.size, installer().len() as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_update_when_current_when_private_404_or_when_it_is_a_draft_or_prerelease() {
    for (release, current) in [
        (rel(json!({})), "1.2.0"),
        (None, "1.0.0"),
        (rel(json!({ "draft": true })), "1.0.0"),
        (rel(json!({ "prerelease": true })), "1.0.0"),
    ] {
        let srv = mock_github(release).await;
        assert!(
            check_for_update(current, &api(&srv))
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_that_is_not_ok_is_an_error_with_the_status() {
    let srv = serve(0, Arc::new(|_| reply(b"{}".to_vec(), 500))).await;
    assert_eq!(
        check_for_update("1.0.0", &api(&srv)).await.unwrap_err(),
        "GitHub a répondu 500"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_installer_is_downloaded_and_verified() {
    let srv = mock_github(rel(json!({}))).await;
    let dir = tmp_dir();
    let info = check_for_update("1.0.0", &api(&srv))
        .await
        .unwrap()
        .unwrap();
    let last = Arc::new(std::sync::Mutex::new((0u64, 0u64)));
    let l = last.clone();
    let file = download_update(&info, &dir, true, move |got, total| {
        *l.lock().unwrap() = (got, total)
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), installer());
    assert_eq!(
        *last.lock().unwrap(),
        (installer().len() as u64, installer().len() as u64)
    );
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        vec![file.file_name().unwrap().to_owned()],
        "no .part left"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_checksum_or_a_missing_one_is_refused_and_leaves_nothing_behind() {
    let srv = mock_github(rel(json!({}))).await;
    let dir = tmp_dir();
    let info = check_for_update("1.0.0", &api(&srv))
        .await
        .unwrap()
        .unwrap();
    let wrong = tachy_core::updater::UpdateInfo {
        sha256: Some("a".repeat(64)),
        ..info.clone()
    };
    let err = download_update(&wrong, &dir, true, |_, _| {})
        .await
        .unwrap_err();
    assert!(err.contains("incorrecte"), "{err}");
    let none = tachy_core::updater::UpdateInfo {
        sha256: None,
        ..info
    };
    let err = download_update(&none, &dir, true, |_, _| {})
        .await
        .unwrap_err();
    assert!(err.contains("absente"), "{err}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_installer_from_an_unexpected_host_is_refused() {
    let dir = tmp_dir();
    let at = |url: &str| tachy_core::updater::UpdateInfo {
        version: "9.9.9".into(),
        url: url.into(),
        size: 0,
        sha256: Some(sha(b"x")),
        page: String::new(),
    };
    for url in [
        "https://evil.example.com/a.exe",
        "http://github.com/a.exe",
        "http://127.0.0.1:9/a.exe", // loopback is only for tests
    ] {
        let err = download_update(&at(url), &dir, false, |_, _| {})
            .await
            .unwrap_err();
        assert!(err.contains("refusée"), "{url}: {err}");
    }
}
