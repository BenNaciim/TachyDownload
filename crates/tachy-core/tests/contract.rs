//! Contract tests (design 7.1): the Rust core against fixtures recorded from the JavaScript engine by
//! `tools/make-fixtures.js`. Settings, `downloads.json`, the snapshots the window receives, the API answers and
//! the French texts must all come out the same.
mod common;
use common::*;
use serde_json::{Map, Value, json};
use std::sync::Arc;
use tachy_core::manager::{Manager, ManagerConfig};
use tachy_core::server::{ServerConfig, listen};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_come_out_the_same_as_in_the_javascript_engine() {
    let f = fixture("settings.json");
    let dir = f["dir"].as_str().unwrap();
    for case in f["cases"].as_array().unwrap() {
        let mut input = case["input"].clone();
        input["dir"] = json!(dir);
        let m = Manager::new(ManagerConfig {
            settings: input.clone(),
            ..ManagerConfig::default()
        });
        let got = serde_json::to_value(m.settings()).unwrap();
        assert_eq!(got, case["output"], "settings for input {input}");
    }
}

/// Differences that belong to a later milestone: the HLS module adds its progress fields and its single segment
/// (M2). Delete this list when M2 lands.
const PENDING_HLS: [&str; 6] = [
    "hlsActive",
    "hlsDone",
    "hlsLive",
    "hlsProgress",
    "hlsTotal",
    "segments",
];

/// A field the JavaScript engine writes as `null` and the Rust one leaves out (or the reverse) reads the same in
/// the window: both are "nothing".
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

/// Compares two task snapshots field by field, so a failure says which field differs.
fn same_task(name: &str, js: &Value, rust: &Value) -> Vec<String> {
    let mut problems = vec![];
    let pending = js["kind"] == "hls";
    let (jo, ro) = (js.as_object().unwrap(), rust.as_object().unwrap());
    let all: std::collections::BTreeSet<&String> = jo.keys().chain(ro.keys()).collect();
    for k in all {
        if pending && PENDING_HLS.contains(&k.as_str()) {
            continue;
        }
        let (j, r) = (
            jo.get(k).unwrap_or(&Value::Null),
            ro.get(k).unwrap_or(&Value::Null),
        );
        if !same_value(j, r) {
            problems.push(format!("{name}: `{k}` is {r} in Rust, {j} in JS"));
        }
    }
    problems
}

fn load_fixture_manager() -> (Manager, std::path::PathBuf, Value) {
    let stored = fixture("downloads.json");
    let store = tmp_dir().join("downloads.json");
    std::fs::write(&store, stored.to_string()).unwrap();
    let m = Manager::new(ManagerConfig {
        store_path: Some(store.clone()),
        settings: json!({ "dir": tmp_dir().to_string_lossy() }),
        tick_ms: 50,
        ..ManagerConfig::default()
    });
    m.load();
    (m, store, stored)
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshots_after_loading_match_what_the_javascript_engine_shows() {
    let (m, _, _) = load_fixture_manager();
    let snaps = fixture("snapshots.json");
    let names = snaps["byName"].as_object().unwrap();
    let rust: Vec<Value> = m
        .list()
        .iter()
        .map(|s| serde_json::to_value(s).unwrap())
        .collect();
    let mut problems = vec![];
    assert_eq!(rust.len(), snaps["snapshots"].as_array().unwrap().len());
    for js in snaps["snapshots"].as_array().unwrap() {
        let id = js["id"].as_str().unwrap();
        let name = names
            .iter()
            .find(|(_, v)| v.as_str() == Some(id))
            .map(|(k, _)| k.as_str())
            .unwrap();
        let Some(r) = rust.iter().find(|r| r["id"] == js["id"]) else {
            problems.push(format!("{name}: not loaded"));
            continue;
        };
        problems.extend(same_task(name, js, r));
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn saving_what_was_loaded_loses_no_field() {
    let (m, store, stored) = load_fixture_manager();
    m.save_now();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&store).unwrap()).unwrap();
    let mut problems = vec![];
    for before in stored["tasks"].as_array().unwrap() {
        let id = before["id"].as_str().unwrap();
        let Some(after) = saved["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == before["id"])
        else {
            problems.push(format!("{id}: lost"));
            continue;
        };
        let (b, a) = (
            before.as_object().unwrap().clone(),
            after.as_object().unwrap().clone(),
        );
        let missing: Map<String, Value> = b
            .into_iter()
            .filter(|(k, v)| !same_value(v, a.get(k).unwrap_or(&Value::Null)))
            .collect();
        for (k, v) in missing {
            problems.push(format!(
                "{id}: `{k}` was {v}, is now {}",
                a.get(&k).unwrap_or(&Value::Null)
            ));
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    // the settings block keeps every key the JavaScript engine wrote
    for (k, v) in stored["settings"].as_object().unwrap() {
        if k != "dir" {
            assert_eq!(saved["settings"].get(k), Some(v), "setting {k}");
        }
    }
    m.shutdown().await;
}

async fn raw(port: u16, req: &Value) -> (u16, Vec<(String, String)>, String) {
    let method = req["method"].as_str().unwrap_or("GET");
    let path = req["path"].as_str().unwrap_or("/");
    let host = req["host"]
        .as_str()
        .map_or(format!("127.0.0.1:{port}"), |h| {
            h.replace("{port}", &port.to_string())
        });
    let body = req["body"].as_str().unwrap_or("");
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(h) = req["headers"].as_object() {
        for (k, v) in h {
            head.push_str(&format!("{k}: {}\r\n", v.as_str().unwrap()));
        }
    }
    if !body.is_empty() {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
        .collect();
    (status, headers, body.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn api_answers_match_the_javascript_server() {
    let f = fixture("api.json");
    let token = f["token"].as_str().unwrap().to_string();
    let m = make_manager(&tmp_dir(), json!({}));
    let srv = listen(
        ServerConfig {
            manager: m.clone(),
            token: Arc::new(move || token.clone()),
            on_added: None,
            on_request: None,
        },
        0,
        1,
    )
    .await
    .unwrap();
    let mut problems = vec![];
    for case in f["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let (status, headers, body) = raw(srv.port(), &case["req"]).await;
        if status as u64 != case["status"].as_u64().unwrap() {
            problems.push(format!("{name}: status {status}, JS {}", case["status"]));
        }
        let got_body: Value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&body).unwrap_or(Value::String(body))
        };
        if got_body != case["body"] {
            problems.push(format!("{name}: body {got_body}, JS {}", case["body"]));
        }
        for (k, v) in case["headers"].as_object().unwrap() {
            let got = headers.iter().find(|(h, _)| h == k).map(|(_, v)| v.clone());
            if got.as_deref() != v.as_str() {
                problems.push(format!("{name}: header {k} is {got:?}, JS {v}"));
            }
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    m.shutdown().await;
}

fn rust_sources(dir: &std::path::Path, out: &mut String) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push_str(&std::fs::read_to_string(p).unwrap());
        }
    }
}

/// A guard against forgetting a message, not proof that each one is used.
#[test]
fn every_french_text_of_the_engine_exists_in_the_rust_sources() {
    let mut src = String::new();
    rust_sources(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut src,
    );
    let missing: Vec<String> = fixture("strings.json")
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        // texts of the video side, which comes with M3 (yt-dlp, tools): remove from this list then
        .filter(|s| !s.contains("playlist") && !s.starts_with("Outils vidéo"))
        .filter(|s| !src.contains(s.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "missing from the Rust sources:\n{missing:#?}"
    );
}
