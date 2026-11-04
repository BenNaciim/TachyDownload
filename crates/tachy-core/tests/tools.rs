//! The video tools: the yt-dlp update check (`test/tools.test.js`) and the installation with its checksum, against a
//! local server instead of GitHub.
mod common;
use common::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tachy_core::tools::{Source, Tools, Update};

const DAY: Duration = Duration::from_secs(20 * 3600);

/// Tools with a copy of our own (or not) whose "latest" and "installed" versions are given; counts installations.
fn fake(
    latest: &'static str,
    installed: &'static str,
    with_copy: bool,
) -> (Tools, Arc<AtomicUsize>) {
    let dir = tmp_dir();
    if with_copy {
        std::fs::write(dir.join("yt-dlp.exe"), "x").unwrap();
    }
    let installs = Arc::new(AtomicUsize::new(0));
    let counter = installs.clone();
    let t = Tools::new(dir)
        .with_latest(move || Box::pin(async move { Ok(latest.to_string()) }))
        .with_installed_version(move |_| Box::pin(async move { installed.to_string() }))
        .with_installer(move |_| {
            let c = counter.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        });
    (t, installs)
}

#[tokio::test]
async fn yt_dlp_an_outdated_own_copy_is_replaced_then_not_checked_again_for_a_while() {
    let (t, installs) = fake("2026.10.26", "2026.01.01", true);
    assert_eq!(
        t.check_ytdlp_update(DAY, || false).await,
        Ok(Update::Updated)
    );
    assert_eq!(installs.load(Ordering::SeqCst), 1);
    assert_eq!(
        t.check_ytdlp_update(DAY, || false).await,
        Ok(Update::Recent)
    );
    assert_eq!(installs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn yt_dlp_an_up_to_date_copy_is_left_alone() {
    let (t, installs) = fake("2026.10.26", "2026.10.26", true);
    assert_eq!(
        t.check_ytdlp_update(DAY, || false).await,
        Ok(Update::Current)
    );
    assert_eq!(installs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn yt_dlp_no_own_copy_or_a_download_in_progress_means_no_update() {
    let (none, _) = fake("v", "old", false);
    assert_eq!(
        none.check_ytdlp_update(DAY, || false).await,
        Ok(Update::Absent)
    );
    let (t, installs) = fake("v", "old", true);
    assert_eq!(t.check_ytdlp_update(DAY, || true).await, Ok(Update::Busy));
    assert_eq!(installs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn yt_dlp_a_failed_lookup_is_an_error_and_leaves_the_copy_as_it_is() {
    let dir = tmp_dir();
    std::fs::write(dir.join("yt-dlp.exe"), "x").unwrap();
    let installs = Arc::new(AtomicUsize::new(0));
    let counter = installs.clone();
    let t = Tools::new(dir)
        .with_latest(|| Box::pin(async { Err("offline".to_string()) }))
        .with_installer(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        });
    assert_eq!(
        t.check_ytdlp_update(DAY, || false).await,
        Err("offline".to_string())
    );
    assert_eq!(installs.load(Ordering::SeqCst), 0);
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A server with a "tool" and its checksum list; `listed` is the hash the list gives (None: the tool is missing
/// from the list).
async fn origin(tool: &'static [u8], listed: Option<String>) -> Server {
    serve(
        0,
        Arc::new(move |req| {
            let reply = |body: Vec<u8>| Resp {
                status: 200,
                headers: vec![],
                to: body.len(),
                body: Arc::new(body),
                from: 0,
                pace: None,
                cut_after: None,
                delay: None,
            };
            match req.path.as_str() {
                "/sums" => reply(
                    match &listed {
                        Some(h) => format!("{}  other.exe\n{h}  yt-dlp.exe\n", "0".repeat(64)),
                        None => format!("{}  other.exe\n", "0".repeat(64)),
                    }
                    .into_bytes(),
                ),
                "/tool" => reply(tool.to_vec()),
                _ => Resp::empty(404),
            }
        }),
    )
    .await
}

fn local(srv: &Server) -> BTreeMap<String, Source> {
    BTreeMap::from([(
        "ytdlp".to_string(),
        Source {
            file: "yt-dlp.exe".into(),
            url: srv.at("/tool"),
            sums: srv.at("/sums"),
            sum_name: Some("yt-dlp.exe".into()),
            sums_re: None,
            zip: false,
        },
    )])
}

#[tokio::test]
async fn an_installed_tool_matches_its_published_checksum_and_leaves_nothing_behind() {
    let srv = origin(b"the tool", Some(hex(b"the tool"))).await;
    let dir = tmp_dir();
    let t = Tools::new(&dir).with_sources(local(&srv));
    let mut events = t.subscribe();
    t.install("ytdlp").await.unwrap();
    assert_eq!(std::fs::read(dir.join("yt-dlp.exe")).unwrap(), b"the tool");
    assert!(!dir.join("yt-dlp.exe.download").exists());
    assert_eq!(t.find("ytdlp"), Some(dir.join("yt-dlp.exe")));
    assert!(t.status().installing.is_none());
    let mut saw_progress = false;
    while let Ok(s) = events.try_recv() {
        saw_progress |= s.installing.is_some();
    }
    assert!(saw_progress, "the window is told while it installs");
}

#[tokio::test]
async fn a_file_that_does_not_match_the_checksum_is_rejected() {
    let srv = origin(b"tampered", Some(hex(b"the tool"))).await;
    let dir = tmp_dir();
    let t = Tools::new(&dir).with_sources(local(&srv));
    assert_eq!(
        t.install("ytdlp").await,
        Err("Somme de contrôle invalide : fichier rejeté".to_string())
    );
    assert!(!dir.join("yt-dlp.exe").exists());
    assert!(!dir.join("yt-dlp.exe.download").exists());
    assert!(
        t.status().installing.is_none(),
        "a failure frees the next installation"
    );
}

#[tokio::test]
async fn without_a_published_checksum_nothing_is_installed() {
    let srv = origin(b"the tool", None).await;
    let dir = tmp_dir();
    let t = Tools::new(&dir).with_sources(local(&srv));
    assert_eq!(
        t.install("ytdlp").await,
        Err("Somme de contrôle introuvable : installation annulée".to_string())
    );
    assert!(!dir.join("yt-dlp.exe").exists());
}

#[tokio::test]
async fn an_unknown_tool_is_refused_and_a_missing_download_is_reported() {
    let t = Tools::new(tmp_dir());
    assert_eq!(t.install("nothing").await, Err("Outil inconnu".to_string()));
    let srv = serve(0, Arc::new(|_| Resp::empty(404))).await;
    let t = Tools::new(tmp_dir()).with_sources(local(&srv));
    // the checksum list is an error page: no hash in it
    assert_eq!(
        t.install("ytdlp").await,
        Err("Somme de contrôle introuvable : installation annulée".to_string())
    );
}
