//! The "Download this?" window: destination, now or later, like IDM's confirmation (`askAboutDownload` and
//! `finishConfirm` in `main.js`). One small window per request; closing it without answering cancels.

use std::path::Path;
use std::sync::atomic::Ordering;

use serde_json::Value;
use tachy_core::manager::AddOpts;
use tachy_core::server::AddRequest;
use tachy_core::util::{filename_from_url, sanitize_filename};
use tachy_core::video::is_playlist_url;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager as _, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use crate::shell::{Confirm, Shell};

/// The window opens 520 x 470 (640 high for a video), like the Electron one. Those were outer sizes: the frame and
/// the title bar take about 16 x 39 pixels of them.
const FRAME: (f64, f64) = (16.0, 39.0);

/// Opens the confirmation window for a download handed over by the browser.
pub fn ask(app: &AppHandle, url: &str, opts: AddOpts, size: u64) {
    let shell = app.state::<Shell>();
    let kind = opts.kind.clone().unwrap_or_else(|| "file".into());
    let guess = opts.filename.clone().filter(|f| !f.is_empty()).or_else(|| {
        if kind == "ytdlp" {
            None
        } else {
            filename_from_url(url)
        }
    });
    let initial_name = sanitize_filename(
        &guess.unwrap_or_else(|| if kind == "hls" { "video" } else { "download" }.to_string()),
    );
    let n = shell.confirm_seq.fetch_add(1, Ordering::Relaxed);
    let label = format!("confirm-{n}");
    let offset = {
        let mut confirms = shell.confirms.lock().unwrap_or_else(|e| e.into_inner());
        let offset = (confirms.len() % 6) as f64 * 28.0;
        let mut req_opts = opts;
        req_opts.kind = Some(kind.clone());
        confirms.insert(
            label.clone(),
            Confirm {
                req: AddRequest {
                    url: url.to_string(),
                    size,
                    opts: req_opts,
                },
                initial_name,
                done: false,
            },
        );
        offset
    };
    let height = if kind == "ytdlp" { 640.0 } else { 470.0 };
    let built =
        WebviewWindowBuilder::new(app, &label, WebviewUrl::App("renderer/confirm.html".into()))
            .title("Tachy Download")
            .inner_size(520.0 - FRAME.0, height - FRAME.1)
            .resizable(false)
            .minimizable(false)
            .maximizable(false)
            .always_on_top(true)
            .visible(false)
            .center()
            .additional_browser_args(&crate::browser_args())
            .on_navigation(crate::is_app_url)
            .on_page_load(move |w, payload| {
                if payload.event() == PageLoadEvent::Finished {
                    if let (Ok(pos), Ok(scale)) = (w.outer_position(), w.scale_factor()) {
                        let d = offset * scale;
                        let _ = w.set_position(PhysicalPosition::new(
                            pos.x + d as i32,
                            pos.y + d as i32,
                        ));
                    }
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            })
            .build();
    if let Err(e) = built {
        eprintln!("confirmation window: {e}");
        shell
            .confirms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&label);
    }
}

/// The window closed (the user's answer or the close button): forget its request.
pub fn forget(app: &AppHandle, label: &str) {
    app.state::<Shell>()
        .confirms
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(label);
}

/// What the user chose: `{ action: 'now' | 'later' | 'cancel', filename, dir, yt }`.
pub async fn answer(app: &AppHandle, label: &str, a: Value) {
    let shell = app.state::<Shell>();
    let req = {
        let mut confirms = shell.confirms.lock().unwrap_or_else(|e| e.into_inner());
        match confirms.get_mut(label) {
            Some(c) if !c.done && a.is_object() => {
                c.done = true;
                Some((c.req.clone(), c.initial_name.clone()))
            }
            _ => None,
        }
    };
    let Some((req, initial_name)) = req else {
        return;
    };
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.close();
    }
    let action = a.get("action").and_then(Value::as_str).unwrap_or("");
    if action != "now" && action != "later" {
        return;
    }
    let kind = req.opts.kind.clone().unwrap_or_else(|| "file".into());
    // Only what the browser told us travels on; nothing else of the request.
    let r = &req.opts;
    let mut opts = AddOpts {
        referer: r.referer.clone(),
        origin: r.origin.clone(),
        page: r.page.clone(),
        cookie: r.cookie.clone(),
        user_agent: r.user_agent.clone(),
        kind: (kind != "file").then(|| kind.clone()),
        sniffed: r.sniffed,
        paused: action == "later",
        ..AddOpts::default()
    };
    let name: String = a
        .get("filename")
        .and_then(Value::as_str)
        .map(|s| s.trim().chars().take(255).collect())
        .unwrap_or_default();
    let had_name = r.filename.as_deref().is_some_and(|f| !f.is_empty());
    if kind != "ytdlp" && !name.is_empty() && (name != initial_name || had_name) {
        opts.filename = Some(name);
    }
    if kind == "ytdlp" {
        opts.yt_opts = a.get("yt").cloned();
        // a manifest named after its page keeps that name
        if r.sniffed && had_name {
            opts.filename = r.filename.clone();
        }
    }
    if let Some(dir) = a.get("dir").and_then(Value::as_str)
        && !dir.is_empty()
        && Path::new(dir).is_absolute()
    {
        opts.dir = Some(dir.to_string());
        opts.own_folder = true;
    }
    let result = if is_playlist_url(&req.url) {
        shell
            .manager
            .add_playlist(&req.url, &opts)
            .await
            .map(|_| ())
    } else {
        shell.manager.add(&req.url, &opts).map(|_| ())
    };
    if let Err(e) = result {
        app.dialog()
            .message(e)
            .title("Tachy Download")
            .kind(MessageDialogKind::Error)
            .show(|_| {});
    }
}
