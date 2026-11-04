//! Needs the Internet: a real public HLS stream, end to end. Run on demand:
//!   cargo test -p tachy-core --test hls_real -- --ignored --nocapture
mod common;
use common::*;
use serde_json::json;
use tachy_core::manager::AddOpts;
use tachy_core::state::Status;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "uses the network"]
async fn a_real_vod_stream_downloads_to_a_playable_transport_stream() {
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "connections": 6, "convertToMp4": false }));
    let url = "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_ts/master.m3u8";
    let id = m
        .add(
            url,
            &AddOpts {
                filename: Some("bipbop".into()),
                ..AddOpts::default()
            },
        )
        .unwrap()
        .state
        .id;
    wait_for("finished", 120000, || is_finished(&m, &id)).await;
    let t = snap(&m, &id);
    assert_eq!(t.state.status, Status::Completed, "{:?}", t.state.error);
    let bytes = std::fs::read(dir.join(&t.state.filename)).unwrap();
    println!(
        "{}: {} bytes, {:?}",
        t.state.filename,
        bytes.len(),
        t.state.hls
    );
    // MPEG-TS packets are 188 bytes long and each starts with 0x47
    assert!(bytes.len() > 100_000);
    assert_eq!(bytes[0], 0x47);
    assert_eq!(bytes.len() % 188, 0);
    m.shutdown().await;
}

/// ffmpeg makes a small HLS stream, the core downloads it and converts it to a regular .mp4 with that same ffmpeg.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ffmpeg in the PATH"]
async fn a_stream_is_converted_to_mp4_when_ffmpeg_is_there() {
    use std::sync::Arc;
    let src = tmp_dir();
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=6:size=320x240:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=6",
        ])
        .args([
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-f",
            "hls",
            "-hls_time",
            "2",
            "-hls_playlist_type",
            "vod",
        ])
        .arg(src.join("v.m3u8"))
        .status()
        .expect("ffmpeg");
    assert!(status.success());
    let files: Arc<std::collections::HashMap<String, Vec<u8>>> = Arc::new(
        std::fs::read_dir(&src)
            .unwrap()
            .map(|e| e.unwrap().path())
            .map(|p| {
                (
                    format!("/{}", p.file_name().unwrap().to_string_lossy()),
                    std::fs::read(p).unwrap(),
                )
            })
            .collect(),
    );
    let srv = serve(
        0,
        Arc::new(
            move |req| match files.get(req.path.split('?').next().unwrap()) {
                Some(b) => {
                    let mut r = Resp::empty(200);
                    r.body = Arc::new(b.clone());
                    r.to = b.len();
                    r
                }
                None => Resp::empty(404),
            },
        ),
    )
    .await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({}));
    m.set_ffmpeg(Arc::new(|| Some(std::path::PathBuf::from("ffmpeg"))));
    let id = m
        .add(
            &srv.at("/v.m3u8"),
            &AddOpts {
                filename: Some("clip".into()),
                ..AddOpts::default()
            },
        )
        .unwrap()
        .state
        .id;
    wait_for("finished", 60000, || is_finished(&m, &id)).await;
    let t = snap(&m, &id);
    assert_eq!(t.state.status, Status::Completed, "{:?}", t.state.error);
    assert_eq!(t.state.filename, "clip.mp4");
    let bytes = std::fs::read(dir.join("clip.mp4")).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp", "a regular mp4");
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        left,
        vec!["clip.mp4".to_string()],
        "no raw file or temporary left"
    );
    assert_eq!(Some(t.downloaded), t.state.size);
    m.shutdown().await;
}
