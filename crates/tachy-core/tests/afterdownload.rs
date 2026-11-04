//! Port of `test/afterdownload.test.js`: filing rules on a finished file, the warning about a copy of a file already
//! received, and unpacking archives.
mod common;
use common::*;
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;
use tachy_core::manager::AddOpts;
use tachy_core::postprocess::{extract_archive, is_archive, system_tar};
use tachy_core::state::Status;

fn named(name: &str) -> AddOpts {
    AddOpts {
        filename: Some(name.into()),
        ..AddOpts::default()
    }
}

fn state_of(m: &tachy_core::manager::Manager, id: &str) -> tachy_core::state::TaskState {
    snap(m, id).state
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rule_sends_the_finished_file_to_its_folder_before_the_type_folders() {
    let body = random_body(80 * 1024);
    let srv = start_server(body.clone(), Opts::default()).await;
    let dir = tmp_dir();
    let m = make_manager(
        &dir,
        json!({ "categories": true, "rules": "127.0.0.1 = Mon site\n.bin = Binaires" }),
    );
    let a = m.add(&srv.url, &named("a.bin")).unwrap().state.id;
    finished(&m, &a).await;
    assert_eq!(
        std::fs::read(dir.join("Mon site").join("a.bin")).unwrap(),
        body,
        "the first matching rule wins"
    );

    let b = m
        .add(
            &srv.url,
            &AddOpts {
                own_folder: true,
                ..named("b.bin")
            },
        )
        .unwrap()
        .state
        .id;
    finished(&m, &b).await;
    assert!(
        dir.join("b.bin").exists(),
        "a folder chosen by hand is left alone"
    );

    m.set_settings(&json!({ "rules": "" })).unwrap();
    let c = m.add(&srv.url, &named("c.bin")).unwrap().state.id;
    finished(&m, &c).await;
    assert!(
        dir.join("Autres").join("c.bin").exists(),
        "no rule: the type folder again"
    );
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_identical_to_one_already_received_is_flagged_a_different_one_of_the_same_size_is_not()
 {
    let (body, other) = (random_body(60 * 1024), random_body(60 * 1024));
    let (s1, s2) = (
        start_server(body, Opts::default()).await,
        start_server(other, Opts::default()).await,
    );
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({}));
    let a = m.add(&s1.url, &named("one.bin")).unwrap().state.id;
    finished(&m, &a).await;
    let b = m.add(&s1.url, &named("two.bin")).unwrap().state.id;
    finished(&m, &b).await;
    wait_for("the copy is flagged", 10_000, || {
        state_of(&m, &b).duplicate_of.is_some()
    })
    .await;
    assert_eq!(
        state_of(&m, &b).duplicate_of,
        Some(dir.join("one.bin").to_string_lossy().into_owned())
    );
    assert_eq!(
        state_of(&m, &a).duplicate_of,
        None,
        "the first copy is the original"
    );

    let c = m.add(&s2.url, &named("three.bin")).unwrap().state.id;
    finished(&m, &c).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await; // time for the check to have run
    assert_eq!(
        state_of(&m, &c).duplicate_of,
        None,
        "same size, other content"
    );

    m.set_settings(&json!({ "duplicates": false })).unwrap();
    let d = m.add(&s1.url, &named("four.bin")).unwrap().state.id;
    finished(&m, &d).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await; // time for the check to have run
    assert_eq!(state_of(&m, &d).duplicate_of, None, "detection off");
    m.shutdown().await;
}

#[test]
fn archives_are_recognised_by_their_extension() {
    for n in [
        "a.zip", "A.ZIP", "b.7z", "c.tar", "d.tar.gz", "e.tgz", "f.tar.xz",
    ] {
        assert!(is_archive(n), "{n}");
    }
    for n in ["a.zipx", "b.rar", "c.txt", "zip"] {
        assert!(!is_archive(n), "{n}");
    }
}

/// A zip with `hello.txt` and `sub/deep.txt`, made by the system tar (the only thing that unpacks it too).
fn make_zip() -> Option<Vec<u8>> {
    let work = tmp_dir();
    std::fs::write(work.join("hello.txt"), "bonjour").unwrap();
    std::fs::create_dir(work.join("sub")).unwrap();
    std::fs::write(work.join("sub").join("deep.txt"), "profond").unwrap();
    let ok = Command::new(system_tar())
        .args(["-a", "-cf"])
        .arg(work.join("pack.zip"))
        .arg("-C")
        .arg(&work)
        .args(["hello.txt", "sub"])
        .status()
        .ok()?
        .success();
    ok.then(|| std::fs::read(work.join("pack.zip")).unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_zip_is_unpacked_into_its_own_folder_when_asked_to() {
    let Some(zip) = make_zip() else {
        eprintln!("tar is not available");
        return;
    };
    let srv = start_server(zip, Opts::default()).await;
    let dir = tmp_dir();
    let m = make_manager(&dir, json!({ "autoExtract": true }));
    let a = m.add(&srv.url, &named("pack.zip")).unwrap().state.id;
    finished(&m, &a).await;
    wait_for("unpacked", 20_000, || {
        state_of(&m, &a).extracted_to.is_some()
    })
    .await;
    let out = PathBuf::from(state_of(&m, &a).extracted_to.unwrap());
    assert_eq!(out, dir.join("pack"));
    assert_eq!(
        std::fs::read_to_string(out.join("hello.txt")).unwrap(),
        "bonjour"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("sub").join("deep.txt")).unwrap(),
        "profond"
    );
    assert!(dir.join("pack.zip").exists(), "the archive itself is kept");

    // a second download of the same name does not merge into the first folder
    let b = m.add(&srv.url, &named("pack.zip")).unwrap().state.id;
    finished(&m, &b).await;
    wait_for("unpacked", 20_000, || {
        state_of(&m, &b).extracted_to.is_some()
    })
    .await;
    assert_ne!(PathBuf::from(state_of(&m, &b).extracted_to.unwrap()), out);

    // setting off: nothing is unpacked
    m.set_settings(&json!({ "autoExtract": false })).unwrap();
    let c = m.add(&srv.url, &named("other.zip")).unwrap().state.id;
    finished(&m, &c).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await; // time for the check to have run
    assert_eq!(state_of(&m, &c).extracted_to, None);
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_archive_reports_the_error_and_leaves_no_empty_folder_behind() {
    let dir = tmp_dir();
    let file = dir.join("broken.zip");
    std::fs::write(&file, random_body(5000)).unwrap();
    assert!(extract_archive(&file, None).await.is_err());
    assert!(
        !dir.join("broken").exists(),
        "the half-made folder is removed"
    );

    let srv = start_server(random_body(5000), Opts::default()).await;
    let m = make_manager(&dir, json!({ "autoExtract": true }));
    let a = m.add(&srv.url, &named("bad.zip")).unwrap().state.id;
    finished(&m, &a).await;
    wait_for("the error is reported", 20_000, || {
        state_of(&m, &a).extract_error.is_some()
    })
    .await;
    let s = state_of(&m, &a);
    assert_eq!(
        s.status,
        Status::Completed,
        "the download itself still counts as done"
    );
    assert_eq!(s.extracting, Some(false));
    m.shutdown().await;
}
