//! THROWAWAY (milestone 0): a fake core so a Tauri window can show the interface before the real engine exists.
//! It mirrors the stand-in used to check the interface by hand: four downloads and one `dm:update` every 250 ms.

use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

const SIZE: u64 = 1_496_157_116;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn stalled(active: u64) -> Value {
    let mut live = vec![1620u64];
    live.extend((0..40).map(|i| 1622 + i));
    json!({
        "id": "h1", "kind": "hls", "status": "downloading",
        "filename": "Série MobLand saison 2 épisode 7.ts", "dir": "C:\\Users\\Public\\Downloads",
        "url": "https://strm13.example.vc/hls2/03/06116/master.m3u8?t=abc", "size": SIZE,
        "downloaded": SIZE * 9 / 10, "speed": 104_000_000u64, "eta": 2, "connections": 8,
        "createdAt": now_ms() - 60_000, "headers": null,
        "segments": [{ "start": 0, "end": SIZE - 1, "pos": SIZE * 9 / 10 }],
        "hlsTotal": 1800, "hlsDone": 1620, "hlsActive": active, "hlsLive": live,
        "hlsProgress": "1620 / 1800 segments",
    })
}

fn plain() -> Value {
    let part = 400_000_000u64 / 8;
    let segments: Vec<Value> = (0..8u64)
        .map(|i| json!({ "start": i * part, "end": (i + 1) * part - 1, "pos": i * part + part * (30 + 8 * i) / 100 }))
        .collect();
    let downloaded: u64 = (0..8u64).map(|i| part * (30 + 8 * i) / 100).sum();
    json!({
        "id": "f1", "status": "downloading", "filename": "archive.zip", "dir": "C:\\Users\\Public\\Downloads",
        "url": "https://files.example.com/archive.zip", "size": 400_000_000u64, "downloaded": downloaded, "rangeOk": true,
        "finalUrl": "https://files.example.com/archive.zip", "connections": 8, "speed": 30_000_000u64, "eta": 6,
        "createdAt": now_ms() - 40_000, "headers": null, "segments": segments,
    })
}

fn done() -> Value {
    json!({
        "id": "c1", "status": "completed", "filename": "rapport.pdf", "dir": "C:\\Users\\Public\\Downloads",
        "url": "https://files.example.com/rapport.pdf", "size": 5_000_000u64, "downloaded": 5_000_000u64, "rangeOk": true,
        "connections": 8, "speed": 0, "eta": null, "createdAt": now_ms() - 90_000, "completedAt": now_ms() - 80_000,
        "headers": null, "segments": null,
    })
}

fn settings() -> Value {
    json!({
        "dir": "", "connections": 8, "maxConcurrent": 3, "speedLimitKBps": 0, "videoMaxHeight": 1080, "convertToMp4": true,
        "categories": false, "autoRetry": true, "adaptive": true, "proxy": "",
        "speedSchedule": { "enabled": false, "from": "08:00", "to": "18:00", "limitKBps": 500 },
        "rules": "", "duplicates": true, "autoExtract": false, "autoStart": false, "closeToTray": true,
        "notifications": true, "confirmDownloads": true, "autoUpdate": true, "ytCookies": "", "afterAll": "none",
        "schedule": { "enabled": false, "from": "01:00", "to": "07:00" },
    })
}

/// `TACHY_FAKE_IDLE=1`: no downloads and no updates, the state the idle measurements compare against (§7.3).
fn idle() -> bool {
    std::env::var_os("TACHY_FAKE_IDLE").is_some()
}

#[tauri::command]
pub fn dm_list() -> Vec<Value> {
    if idle() {
        Vec::new()
    } else {
        vec![stalled(1), plain(), done()]
    }
}

#[tauri::command]
pub fn dm_state() -> Value {
    json!({ "settings": settings(), "port": 6800, "version": "2.0.0-m0", "update": null, "token": "x", "glass": true })
}

#[tauri::command]
pub fn dm_tools_status() -> Value {
    json!({ "ytdlp": { "found": false }, "deno": { "found": false }, "ffmpeg": { "found": false } })
}

#[tauri::command]
pub fn dm_file_icon(_id: String) -> Option<String> {
    None
}

#[tauri::command]
pub fn dm_logins_list() -> Vec<Value> {
    Vec::new()
}

#[tauri::command]
pub fn dm_settings_set(patch: Value) -> Value {
    let mut merged = settings();
    if let (Some(base), Some(over)) = (merged.as_object_mut(), patch.as_object()) {
        for (k, v) in over {
            base.insert(k.clone(), v.clone());
        }
    }
    merged
}

#[tauri::command]
pub fn dm_copy_token() -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub fn dm_redownload(_id: String) -> Result<(), String> {
    Ok(())
}

pub fn start_ticker(app: AppHandle) {
    if idle() {
        return;
    }
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let active = 1 + (now_ms() / 1000) % 3;
            if app.emit("dm:update", stalled(active)).is_err() {
                break;
            }
        }
    });
}
