//! The commands the page calls (`ipcMain.handle('dm:...')` in `main.js`, named `dm_...` here). Every one validates
//! what it gets and fails with the same French message as before.

use serde_json::{Value, json};
use tachy_core::manager::AddOpts;
use tachy_core::probe::is_web_page;
use tachy_core::state::Snapshot;
use tachy_core::tools::ToolsStatus;
use tachy_core::video::{is_playlist_url, is_video_page};
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::icons::Target;
use crate::shell::{Shell, task_path};

type Res<T> = Result<T, String>;

const INVALID: &str = "Paramètre invalide";

fn short(v: String, max: usize) -> Res<String> {
    if v.chars().count() > max {
        Err(INVALID.into())
    } else {
        Ok(v)
    }
}

fn is_http(link: &str) -> bool {
    let l = link.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

#[tauri::command]
pub async fn dm_list(shell: State<'_, Shell>) -> Res<Vec<Snapshot>> {
    Ok(shell.manager.list())
}

#[tauri::command]
pub async fn dm_state(shell: State<'_, Shell>) -> Res<Value> {
    let update = shell
        .pending_update
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Ok(json!({
        "settings": shell.manager.settings(),
        "port": shell.port.load(std::sync::atomic::Ordering::Relaxed),
        "version": shell.version,
        "update": update,
        "token": shell.token,
        "glass": shell.glass,
    }))
}

#[tauri::command]
pub async fn dm_add(
    app: AppHandle,
    shell: State<'_, Shell>,
    urls: Value,
    opts: Option<Value>,
) -> Res<Value> {
    let urls = match urls.as_array() {
        Some(a) if !a.is_empty() && a.len() <= 200 => a,
        _ => return Err("Liste de liens invalide".into()),
    };
    let start_at = opts
        .as_ref()
        .and_then(|o| o.get("startAt"))
        .and_then(Value::as_f64)
        .filter(|n| *n != 0.0 && n.is_finite())
        .map(|n| n as u64);
    let asked = opts
        .as_ref()
        .and_then(|o| o.get("kind"))
        .and_then(Value::as_str)
        == Some("ytdlp");
    let (mut added, mut errors) = (0usize, Vec::<Value>::new());
    for raw in urls {
        let one = async {
            let raw = raw
                .as_str()
                .filter(|s| s.chars().count() <= 8192)
                .ok_or_else(|| INVALID.to_string())?;
            let link = raw.trim();
            // A page of a site the app does not know (it answers with HTML) is left to yt-dlp, which knows many more.
            let ytdlp = asked
                || (is_http(link)
                    && !is_playlist_url(link)
                    && !is_video_page(link)
                    && is_web_page(&shell.manager.net(), link, &Vec::new()).await);
            let kind = ytdlp.then(|| "ytdlp".to_string());
            if is_playlist_url(raw) {
                let o = AddOpts {
                    start_at,
                    ..AddOpts::default()
                };
                added += shell.manager.add_playlist(raw, &o).await?.len();
            } else if shell.manager.settings().confirm_downloads
                && start_at.is_none()
                && (ytdlp || is_video_page(raw))
            {
                // A video page: let the user pick the quality, the audio language and the folder first.
                if !is_http(raw) {
                    return Err("Adresse invalide".to_string());
                }
                crate::confirm::ask(
                    &app,
                    link,
                    AddOpts {
                        kind: Some("ytdlp".into()),
                        ..AddOpts::default()
                    },
                    0,
                );
                added += 1;
            } else {
                shell.manager.add(
                    raw,
                    &AddOpts {
                        start_at,
                        kind,
                        ..AddOpts::default()
                    },
                )?;
                added += 1;
            }
            Ok::<(), String>(())
        };
        if let Err(error) = one.await {
            let url: String = raw.as_str().unwrap_or("").chars().take(200).collect();
            errors.push(json!({ "url": url, "error": error }));
        }
    }
    Ok(json!({ "added": added, "errors": errors }))
}

#[tauri::command]
pub async fn dm_open_page(app: AppHandle, shell: State<'_, Shell>, id: String) -> Res<()> {
    let id = short(id, 100)?;
    let page = shell
        .manager
        .list()
        .into_iter()
        .find(|t| t.state.id == id)
        .map(|t| t.state.page_url)
        .filter(|p| is_http(p))
        .ok_or("Page d’origine inconnue")?;
    app.opener()
        .open_url(page, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dm_file_icon(shell: State<'_, Shell>, id: String) -> Res<Option<String>> {
    let id = short(id, 100)?;
    let Some(t) = shell.manager.list().into_iter().find(|t| t.state.id == id) else {
        return Ok(None);
    };
    Ok(shell
        .icons
        .file_icon(Target {
            filename: &t.state.filename,
            dir: &t.state.dir,
            status: t.state.status,
        })
        .await)
}

#[tauri::command]
pub async fn dm_update_check(app: AppHandle, shell: State<'_, Shell>) -> Res<Value> {
    let found = crate::update::look_for_update(&app).await?;
    Ok(json!({ "current": shell.version, "update": found }))
}

#[tauri::command]
pub async fn dm_update_install() -> Res<()> {
    Err("Aucune mise à jour à installer".into())
}

#[tauri::command]
pub async fn dm_tools_status(shell: State<'_, Shell>) -> Res<ToolsStatus> {
    Ok(shell.tools.status())
}

#[tauri::command]
pub async fn dm_tools_install(shell: State<'_, Shell>, name: String) -> Res<ToolsStatus> {
    if !["ytdlp", "ffmpeg", "deno"].contains(&name.as_str()) {
        return Err("Outil inconnu".into());
    }
    shell.tools.install(&name).await?;
    Ok(shell.tools.status())
}

#[tauri::command]
pub async fn dm_pause(shell: State<'_, Shell>, id: String) -> Res<()> {
    shell.manager.pause(&short(id, 100)?).await
}

#[tauri::command]
pub async fn dm_resume(shell: State<'_, Shell>, id: String) -> Res<()> {
    shell.manager.resume(&short(id, 100)?)
}

#[tauri::command]
pub async fn dm_redownload(shell: State<'_, Shell>, id: String) -> Res<()> {
    shell.manager.redownload(&short(id, 100)?).map(|_| ())
}

#[tauri::command]
pub async fn dm_set_limit(shell: State<'_, Shell>, id: String, kbps: f64) -> Res<()> {
    shell
        .manager
        .set_task_limit(&short(id, 100)?, kbps)
        .map(|_| ())
}

#[tauri::command]
pub async fn dm_remove(shell: State<'_, Shell>, id: String, delete_file: Option<bool>) -> Res<()> {
    shell
        .manager
        .remove(&short(id, 100)?, delete_file == Some(true))
        .await
}

#[tauri::command]
pub async fn dm_reveal(app: AppHandle, shell: State<'_, Shell>, id: String) -> Res<()> {
    let path = task_path(&shell, &short(id, 100)?)?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dm_open(app: AppHandle, shell: State<'_, Shell>, id: String) -> Res<()> {
    let path = task_path(&shell, &short(id, 100)?)?;
    if path.extension().is_some_and(|e| e == "part") {
        return Err("Le téléchargement n’est pas terminé".into());
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dm_settings_set(app: AppHandle, shell: State<'_, Shell>, patch: Value) -> Res<Value> {
    if !patch.is_object() {
        return Err("Paramètres invalides".into());
    }
    // The manager keeps only the settings it knows, so the patch goes straight through.
    let next = shell.manager.set_settings(&patch)?;
    crate::autostart::apply(&app, next.auto_start);
    serde_json::to_value(next).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dm_pick_dir(shell: State<'_, Shell>, window: WebviewWindow) -> Res<Option<String>> {
    Ok(pick_dir(&window, shell.manager.settings().dir))
}

fn pick_dir(window: &WebviewWindow, start: String) -> Option<String> {
    let mut dialog = window.dialog().file().set_parent(window);
    if !start.is_empty() {
        dialog = dialog.set_directory(start);
    }
    dialog
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn dm_copy_token(app: AppHandle, shell: State<'_, Shell>) -> Res<()> {
    app.clipboard()
        .write_text(shell.token.clone())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dm_copy_diagnostic(app: AppHandle) -> Res<()> {
    crate::menu::copy_diagnostic(&app)
}

// Site logins arrive with the data migration (milestone 5).

#[tauri::command]
pub async fn dm_logins_list() -> Res<Vec<Value>> {
    Ok(Vec::new())
}

#[tauri::command]
pub async fn dm_logins_save(_entry: Value) -> Res<Value> {
    Err("Les identifiants de site ne sont pas encore disponibles".into())
}

#[tauri::command]
pub async fn dm_logins_remove(_host: String) -> Res<Vec<Value>> {
    Ok(Vec::new())
}

// ---- the "Download this?" window ----------------------------------------------------------------------------

#[tauri::command]
pub async fn confirm_init(shell: State<'_, Shell>, window: WebviewWindow) -> Res<Option<Value>> {
    let (req, name) = {
        let confirms = shell.confirms.lock().unwrap_or_else(|e| e.into_inner());
        match confirms.get(window.label()) {
            Some(c) => (c.req.clone(), c.initial_name.clone()),
            None => return Ok(None),
        }
    };
    let host = url::Url::parse(&req.url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    let icon = shell
        .icons
        .file_icon(Target {
            filename: if name.is_empty() { "file.bin" } else { &name },
            dir: "",
            status: tachy_core::state::Status::Queued,
        })
        .await;
    Ok(Some(json!({
        "url": req.url,
        "kind": req.opts.kind.as_deref().unwrap_or("file"),
        "size": req.size,
        "sniffed": req.opts.sniffed,
        "filename": name,
        "dir": shell.manager.settings().dir,
        "host": host,
        "icon": icon,
    })))
}

#[tauri::command]
pub async fn confirm_category(
    shell: State<'_, Shell>,
    window: WebviewWindow,
    name: Option<String>,
) -> Res<Option<&'static str>> {
    let known = shell
        .confirms
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(window.label());
    Ok(match name {
        Some(n) if known && shell.manager.settings().categories => {
            Some(tachy_core::filetypes::category_for(&n))
        }
        _ => None,
    })
}

#[tauri::command]
pub async fn confirm_pick_dir(
    shell: State<'_, Shell>,
    window: WebviewWindow,
    current: Option<String>,
) -> Res<Option<String>> {
    let known = shell
        .confirms
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(window.label());
    if !known {
        return Ok(None);
    }
    let start = current
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| shell.manager.settings().dir);
    Ok(pick_dir(&window, start))
}

#[tauri::command]
pub async fn confirm_probe(shell: State<'_, Shell>, window: WebviewWindow) -> Res<Value> {
    let req = {
        let confirms = shell.confirms.lock().unwrap_or_else(|e| e.into_inner());
        confirms.get(window.label()).map(|c| c.req.clone())
    };
    let unavailable = |error: Option<&str>| match error {
        Some(e) => json!({ "unavailable": true, "error": e }),
        None => json!({ "unavailable": true }),
    };
    let Some(req) = req else {
        return Ok(unavailable(None));
    };
    if req.opts.kind.as_deref() != Some("ytdlp") || is_playlist_url(&req.url) {
        return Ok(unavailable(None));
    }
    let Some(exe) = shell.tools.find("ytdlp") else {
        return Ok(unavailable(Some(
            "yt-dlp n’est pas installé : ouvrez Paramètres → Vidéos.",
        )));
    };
    let yt = tachy_core::video::YtdlpBin {
        exe,
        launch: Default::default(),
    };
    let settings = shell.manager.settings();
    let deno = shell.tools.find("deno");
    let headers = tachy_core::state::ReplayHeaders {
        referer: req.opts.referer.clone().unwrap_or_default(),
        origin: req.opts.origin.clone().unwrap_or_default(),
        cookie: String::new(),
        user_agent: req.opts.user_agent.clone().unwrap_or_default(),
    };
    let probe = tachy_core::video::probe_video(
        &yt,
        &req.url,
        &tachy_core::video::ProbeOpts {
            deno: deno.as_deref(),
            cookies: &settings.yt_cookies,
            proxy: &settings.proxy,
            headers: Some(&headers),
        },
    )
    .await;
    Ok(match probe {
        Ok(p) => {
            let mut v = serde_json::to_value(p).map_err(|e| e.to_string())?;
            v["ffmpeg"] = json!(shell.tools.find("ffmpeg").is_some());
            v["maxHeight"] = json!(settings.video_max_height);
            v
        }
        Err(e) => unavailable(Some(&e)),
    })
}

#[tauri::command]
pub async fn confirm_answer(app: AppHandle, window: WebviewWindow, answer: Value) -> Res<()> {
    crate::confirm::answer(&app, window.label(), answer).await;
    Ok(())
}
