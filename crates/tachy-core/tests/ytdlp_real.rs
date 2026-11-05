//! Against the real yt-dlp, the real network and a real (short, public) video. Ignored by default; run with
//!   cargo test -p tachy-core --test ytdlp_real -- --ignored --nocapture
//! It uses the tools of the Tachy Download data folder (`%APPDATA%\Tachy Download\tools`) or whatever is on PATH.
mod common;
use common::*;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tachy_core::manager::{AddOpts, Manager, ManagerConfig};
use tachy_core::state::Status;
use tachy_core::tools::Tools;
use tachy_core::video::{ProbeOpts, YtdlpBin, probe_video};

/// A short public video (3.5 minutes; at 360p a few MB).
const CLIP: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

fn tools() -> Arc<Tools> {
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    Arc::new(Tools::new(
        PathBuf::from(appdata).join("Tachy Download").join("tools"),
    ))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs yt-dlp and the network"]
async fn a_real_video_is_probed() {
    let t = tools();
    let yt = YtdlpBin {
        exe: t.find("ytdlp").expect("yt-dlp is installed"),
        launch: Default::default(),
    };
    let deno = t.find("deno");
    let p = probe_video(
        &yt,
        CLIP,
        &ProbeOpts {
            deno: deno.as_deref(),
            ..ProbeOpts::default()
        },
    )
    .await
    .unwrap();
    println!("{p:?}");
    assert!(p.title.to_lowercase().contains("astley"));
    assert!(!p.heights.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs yt-dlp and the network"]
async fn a_real_video_is_downloaded_with_progress_and_a_clean_end() {
    let t = tools();
    let (dir, data) = (tmp_dir(), tmp_dir());
    let finder = t.clone();
    let m = Manager::new(ManagerConfig {
        store_path: Some(data.join("downloads.json")),
        settings: json!({ "dir": dir.to_string_lossy(), "videoMaxHeight": 360 }),
        tick_ms: 100,
        tools: Some(Arc::new(move |n| finder.find(n))),
        ..ManagerConfig::default()
    });
    m.start();
    let id = m.add(CLIP, &AddOpts::default()).unwrap().state.id;
    wait_for("the video ends", 180_000, || is_finished(&m, &id)).await;
    let done = snap(&m, &id);
    println!(
        "{:?} {} {:?} {:?}",
        done.state.status, done.state.filename, done.state.size, done.state.error
    );
    println!(
        "{}",
        std::fs::read_to_string(data.join("ytdlp-last.log")).unwrap_or_default()
    );
    assert_eq!(
        done.state.status,
        Status::Completed,
        "{:?}",
        done.state.error
    );
    let file = dir.join(&done.state.filename);
    assert!(file.exists());
    assert!(std::fs::metadata(&file).unwrap().len() > 100_000);
    m.shutdown().await;
}
