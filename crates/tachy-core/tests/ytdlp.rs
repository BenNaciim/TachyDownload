//! A video through the whole engine with a stand-in for yt-dlp (`tests/fixtures/fake-ytdlp.js`, started through
//! node): arguments, names, diagnostics, messages, redownload, and the plain-file shortcut.
mod common;
use common::*;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tachy_core::ctx::YtdlpLaunch;
use tachy_core::manager::{AddOpts, Manager, ManagerConfig};
use tachy_core::state::{Snapshot, Status};

const URL: &str = "https://www.youtube.com/watch?v=abcdefghijk";

fn fake() -> String {
    format!(
        "{}/tests/fixtures/fake-ytdlp.js",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn launch(env: &[(&str, &str)]) -> YtdlpLaunch {
    YtdlpLaunch {
        prefix: vec![fake()],
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

fn manager(dir: &Path, data: &Path, settings: serde_json::Value, env: &[(&str, &str)]) -> Manager {
    let mut s = settings;
    s["dir"] = json!(dir.to_string_lossy());
    let m = Manager::new(ManagerConfig {
        store_path: Some(data.join("downloads.json")),
        settings: s,
        tick_ms: 50,
        tools: Some(Arc::new(|n| (n == "ytdlp").then(|| PathBuf::from("node")))),
        ytdlp_launch: Some(launch(env)),
        ..ManagerConfig::default()
    });
    m.start();
    m
}

struct Run {
    task: Snapshot,
    dir: PathBuf,
    args: Vec<String>,
    diag: Option<String>,
}

#[derive(Default)]
struct Add {
    url: Option<&'static str>,
    opts: AddOpts,
    before_file: Option<(&'static str, &'static str)>,
}

fn args_of(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Runs one video through the real engine with the stand-in yt-dlp and returns what happened.
async fn run(mode: &str, settings: serde_json::Value, task_limit: f64, add: Add) -> Run {
    let dir = tmp_dir();
    let data = tmp_dir();
    if let Some((name, text)) = add.before_file {
        std::fs::write(dir.join(name), text).unwrap();
    }
    let args_file = data.join("args.json");
    let m = manager(
        &dir,
        &data,
        settings,
        &[
            ("FAKE_MODE", mode),
            ("FAKE_ARGS_FILE", &args_file.to_string_lossy()),
        ],
    );
    let paused = task_limit > 0.0;
    let id = m
        .add(
            add.url.unwrap_or(URL),
            &AddOpts {
                kind: Some("ytdlp".into()),
                paused,
                ..add.opts
            },
        )
        .unwrap()
        .state
        .id;
    if paused {
        m.set_task_limit(&id, task_limit).unwrap();
        m.resume(&id).unwrap();
    }
    wait_for("the video ends", 20000, || is_finished(&m, &id)).await;
    let task = snap(&m, &id);
    let diag = std::fs::read_to_string(data.join("ytdlp-last.log")).ok();
    m.shutdown().await;
    Run {
        task,
        dir,
        args: args_of(&args_file),
        diag,
    }
}

fn arg_after(args: &[String], flag: &str) -> String {
    let i = args.iter().position(|a| a == flag).unwrap();
    args[i + 1].clone()
}

fn manifest() -> Add {
    Add {
        url: Some("https://cdn.example/v/manifest.mpd"),
        opts: AddOpts {
            sniffed: true,
            filename: Some("Série - S01E04".into()),
            referer: Some("https://player.example/e/1".into()),
            origin: Some("https://player.example".into()),
            ..AddOpts::default()
        },
        before_file: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_normal_run_completes_keeps_the_file_and_always_asks_for_a_single_video() {
    let r = run("ok", json!({}), 0.0, Add::default()).await;
    let s = &r.task.state;
    assert_eq!(s.status, Status::Completed, "{:?}", s.error);
    assert_eq!(s.filename, "Episode [abcdefghijk].mp4");
    assert!(r.dir.join(&s.filename).exists());
    assert_eq!(s.size, Some(2 * 1024 * 1024));
    for flag in ["--no-playlist", "--no-simulate", "--progress", "--encoding"] {
        assert!(
            r.args.iter().any(|a| a == flag),
            "{flag} is passed to yt-dlp"
        );
    }
    assert!(
        r.diag.unwrap().contains("Code de sortie : 0"),
        "a diagnostic is written even on success"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_manifest_found_by_the_extension_is_named_after_its_page_and_gets_the_headers_the_browser_sent()
 {
    let r = run("ok", json!({}), 0.0, manifest()).await;
    let s = &r.task.state;
    assert_eq!(s.status, Status::Completed, "{:?}", s.error);
    assert_eq!(s.filename, "Série - S01E04.mp4");
    assert!(r.dir.join("Série - S01E04.mp4").exists());
    assert_eq!(arg_after(&r.args, "-o"), "Série - S01E04.%(ext)s");
    assert_eq!(
        arg_after(&r.args, "--referer"),
        "https://player.example/e/1"
    );
    assert!(r.args.iter().any(|a| a == "Origin:https://player.example"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_manifest_named_after_its_page_never_overwrites_a_file_that_is_already_there() {
    let r = run(
        "ok",
        json!({}),
        0.0,
        Add {
            before_file: Some(("Série - S01E04.mp4", "older")),
            ..manifest()
        },
    )
    .await;
    assert_eq!(
        r.task.state.status,
        Status::Completed,
        "{:?}",
        r.task.state.error
    );
    assert_eq!(r.task.state.filename, "Série - S01E04 (1).mp4");
    assert_eq!(
        std::fs::read_to_string(r.dir.join("Série - S01E04.mp4")).unwrap(),
        "older"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_percent_sign_in_the_page_title_is_not_read_as_a_yt_dlp_template_field() {
    let mut add = manifest();
    add.opts.filename = Some("100% vrai".into());
    let r = run("ok", json!({}), 0.0, add).await;
    assert_eq!(arg_after(&r.args, "-o"), "100%% vrai.%(ext)s");
    assert_eq!(r.task.state.filename, "100% vrai.mp4");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_printed_path_that_lost_a_character_is_found_again_by_the_video_id() {
    let r = run("renamed", json!({}), 0.0, Add::default()).await;
    assert_eq!(
        r.task.state.status,
        Status::Completed,
        "{:?}",
        r.task.state.error
    );
    assert_eq!(r.task.state.filename, "Episode [abcdefghijk].mp4");
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_code_0_without_any_file_fails_with_a_diagnostic_instead_of_a_silent_success() {
    let r = run("nofile", json!({}), 0.0, Add::default()).await;
    assert_eq!(r.task.state.status, Status::Error);
    let error = r.task.state.error.unwrap();
    assert!(
        error.contains("code 0") || error.contains("n’a pas pu"),
        "{error}"
    );
    let diag = r.diag.unwrap();
    assert!(diag.contains("introuvable"));
    assert!(diag.contains("ghost.mp4"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_known_youtube_refusal_is_explained_and_the_diagnostic_keeps_the_raw_message() {
    let r = run("fail", json!({}), 0.0, Add::default()).await;
    assert_eq!(r.task.state.status, Status::Error);
    assert!(r.task.state.error.unwrap().contains("robot"));
    let diag = r.diag.unwrap();
    assert!(diag.contains("Sign in to confirm"));
    assert!(diag.contains("Code de sortie : 1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn with_type_folders_on_the_video_lands_in_its_folder() {
    let r = run("ok", json!({ "categories": true }), 0.0, Add::default()).await;
    assert_eq!(
        r.task.state.status,
        Status::Completed,
        "{:?}",
        r.task.state.error
    );
    assert!(
        r.dir
            .join("Vidéos")
            .join("Episode [abcdefghijk].mp4")
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_lower_of_the_shared_and_the_own_speed_limit_goes_to_yt_dlp() {
    let r = run(
        "ok",
        json!({ "speedLimitKBps": 500 }),
        200.0,
        Add::default(),
    )
    .await;
    assert_eq!(
        r.task.state.status,
        Status::Completed,
        "{:?}",
        r.task.state.error
    );
    assert_eq!(arg_after(&r.args, "--limit-rate"), "200K");
    let s = run(
        "ok",
        json!({ "speedLimitKBps": 100 }),
        400.0,
        Add::default(),
    )
    .await;
    assert_eq!(arg_after(&s.args, "--limit-rate"), "100K");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_file_site_is_fetched_by_the_app_itself_over_several_connections() {
    let body = Arc::new(vec![9u8; 5 * MB]);
    let origin = {
        let body = body.clone();
        serve(0, Arc::new(move |req| ranged(&body, req, true, None))).await
    };
    let (dir, data) = (tmp_dir(), tmp_dir());
    let m = manager(
        &dir,
        &data,
        json!({ "connections": 4 }),
        &[("FAKE_DIRECT_URL", &origin.url)],
    );
    let id = m
        .add(
            "https://videos.example.org/watch/42",
            &AddOpts {
                kind: Some("ytdlp".into()),
                ..AddOpts::default()
            },
        )
        .unwrap()
        .state
        .id;
    wait_for("the video ends", 20000, || is_finished(&m, &id)).await;
    let t = snap(&m, &id);
    assert_eq!(t.state.status, Status::Completed, "{:?}", t.state.error);
    assert_eq!(t.state.filename, "Direct clip.mp4");
    assert_eq!(std::fs::read(dir.join("Direct clip.mp4")).unwrap(), *body);
    assert!(
        t.state.segments.as_ref().is_some_and(|s| s.len() > 1) || t.state.direct == Some(true),
        "the app split the file itself"
    );
    m.shutdown().await;
}

/// Downloads a video, then asks for it again; the stand-in fills the second copy with another byte.
async fn run_twice(
    url: &'static str,
    opts: AddOpts,
    extra: &[(&str, &str)],
) -> (Snapshot, PathBuf, Vec<String>) {
    let (dir, data) = (tmp_dir(), tmp_dir());
    let args_file = data.join("args.json");
    let args_path = args_file.to_string_lossy().into_owned();
    let mut env = vec![
        ("FAKE_MODE", "ok"),
        ("FAKE_ARGS_FILE", args_path.as_str()),
        ("FAKE_FILL", "7"),
    ];
    env.extend_from_slice(extra);
    let m = manager(&dir, &data, json!({}), &env);
    let id = m
        .add(
            url,
            &AddOpts {
                kind: Some("ytdlp".into()),
                ..opts
            },
        )
        .unwrap()
        .state
        .id;
    wait_for("the first copy", 20000, || is_finished(&m, &id)).await;
    let first = snap(&m, &id);
    assert_eq!(
        first.state.status,
        Status::Completed,
        "{:?}",
        first.state.error
    );
    env[2] = ("FAKE_FILL", "9");
    m.set_ytdlp_launch(launch(&env));
    m.redownload(&id).unwrap();
    wait_for("the second copy", 20000, || {
        is_finished(&m, &id) && snap(&m, &id).state.replaces.is_none()
    })
    .await;
    let task = snap(&m, &id);
    m.shutdown().await;
    // No arguments file when the app fetched the plain file itself: yt-dlp only answered the probe.
    (task, dir, args_of(&args_file))
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn a_video_downloaded_again_is_overwritten_in_place_yt_dlp_is_told_to_and_the_name_stays() {
    let (task, dir, args) = run_twice(URL, AddOpts::default(), &[]).await;
    assert_eq!(
        task.state.status,
        Status::Completed,
        "{:?}",
        task.state.error
    );
    assert!(
        args.iter().any(|a| a == "--force-overwrites"),
        "without it yt-dlp answers \"already downloaded\" and does nothing"
    );
    assert_eq!(task.state.filename, "Episode [abcdefghijk].mp4");
    assert_eq!(names(&dir), ["Episode [abcdefghijk].mp4"]);
    assert_eq!(
        std::fs::read(dir.join(&task.state.filename)).unwrap()[0],
        9,
        "it is the new copy"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_video_named_after_its_page_keeps_that_name_when_downloaded_again() {
    let m = manifest();
    let (task, dir, _) = run_twice(m.url.unwrap(), m.opts, &[]).await;
    assert_eq!(
        task.state.status,
        Status::Completed,
        "{:?}",
        task.state.error
    );
    assert_eq!(names(&dir), ["Série - S01E04.mp4"]);
    assert_eq!(std::fs::read(dir.join("Série - S01E04.mp4")).unwrap()[0], 9);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_file_video_fetched_by_the_app_keeps_its_name_when_downloaded_again() {
    let body = Arc::new(vec![5u8; 3 * MB]);
    let origin = {
        let body = body.clone();
        serve(0, Arc::new(move |req| ranged(&body, req, true, None))).await
    };
    let (task, dir, _) = run_twice(
        "https://videos.example.org/watch/42",
        AddOpts::default(),
        &[("FAKE_DIRECT_URL", &origin.url)],
    )
    .await;
    assert_eq!(
        task.state.status,
        Status::Completed,
        "{:?}",
        task.state.error
    );
    assert_eq!(
        names(&dir),
        ["Direct clip.mp4"],
        "no \"(1)\" copy beside it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_yt_dlp_task_without_the_tool_fails_with_an_actionable_message() {
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({}));
    let id = m
        .add(
            "https://www.youtube.com/watch?v=jNQXAC9IJ2w",
            &AddOpts::default(),
        )
        .unwrap();
    assert_eq!(id.state.kind, tachy_core::state::Kind::Ytdlp);
    let id = id.state.id;
    wait_for("the video ends", 20000, || is_finished(&m, &id)).await;
    assert!(snap(&m, &id).state.error.unwrap().contains("Outils vidéo"));
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn add_playlist_without_yt_dlp_explains_what_to_install() {
    let m = make_manager(&tmp_dir(), json!({}));
    let err = m
        .add_playlist(
            "https://www.youtube.com/playlist?list=PLabc",
            &AddOpts::default(),
        )
        .await
        .unwrap_err();
    assert!(err.contains("Outils vidéo"));
    m.shutdown().await;
}

fn bin(env: &[(&str, &str)]) -> tachy_core::video::YtdlpBin {
    tachy_core::video::YtdlpBin {
        exe: PathBuf::from("node"),
        launch: launch(env),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_video_is_probed_for_its_heights_and_languages() {
    use tachy_core::video::{ProbeOpts, probe_video};
    let p = probe_video(&bin(&[("FAKE_PROBE", "1")]), URL, &ProbeOpts::default())
        .await
        .unwrap();
    assert_eq!(p.title, "Clip");
    assert_eq!(p.duration, 61.0);
    assert_eq!(p.heights, [1080, 720]);
    assert_eq!(p.langs.len(), 1);
    assert_eq!(
        (p.langs[0].code.as_str(), p.langs[0].label.as_str()),
        ("fr", "Français")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_probe_that_fails_says_why_in_french() {
    use tachy_core::video::{ProbeOpts, probe_video};
    let err = probe_video(&bin(&[("FAKE_MODE", "fail")]), URL, &ProbeOpts::default())
        .await
        .unwrap_err();
    assert!(err.contains("robot"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_playlist_becomes_one_video_per_entry_in_a_folder_named_after_it() {
    let (dir, data) = (tmp_dir(), tmp_dir());
    let m = manager(&dir, &data, json!({}), &[("FAKE_MODE", "ok")]);
    let added = m
        .add_playlist(
            "https://www.youtube.com/playlist?list=PLabc",
            &AddOpts {
                paused: true,
                ..AddOpts::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(added.len(), 2);
    let folder = dir.join("Ma liste").to_string_lossy().into_owned();
    for (t, name) in added.iter().zip(["Premier.mp4", "Second.mp4"]) {
        assert_eq!(t.state.kind, tachy_core::state::Kind::Ytdlp);
        assert_eq!(t.state.dir, folder);
        assert_eq!(t.state.filename, name);
        assert!(t.state.own_folder);
    }
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_playlist_is_refused() {
    let m = manager(
        &tmp_dir(),
        &tmp_dir(),
        json!({}),
        &[("FAKE_PLAYLIST", "empty")],
    );
    let err = m
        .add_playlist(
            "https://www.youtube.com/playlist?list=PLabc",
            &AddOpts::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err, "Cette playlist est vide ou privée");
    m.shutdown().await;
}
