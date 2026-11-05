//! The window's menu bar and what each entry does. The layout comes from `tachy_core::menu` (checked there); this
//! turns it into native menus and runs the chosen action.

use serde_json::json;
use tachy_core::menu::{self as spec, Action, Item, Role};
use tachy_core::state::Status;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager as _};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

use crate::shell::{PROJECT_URL, Shell, attention, main_window, send, show_window};

/// What a native entry is tied to.
#[derive(Debug, Clone)]
pub enum Entry {
    Core(Action),
    Role(Role),
}

fn accelerator(role: Role) -> Option<&'static str> {
    match role {
        Role::ZoomIn => Some("CmdOrCtrl+="),
        Role::ZoomOut => Some("CmdOrCtrl+-"),
        Role::ResetZoom => Some("CmdOrCtrl+0"),
        Role::ToggleFullscreen => Some("F11"),
        Role::ToggleDevTools => Some("CmdOrCtrl+Shift+I"),
    }
}

fn fill(
    app: &AppHandle,
    menu: &Submenu<tauri::Wry>,
    items: &[Item],
    entries: &mut Vec<Entry>,
) -> tauri::Result<()> {
    for item in items {
        match item {
            Item::Separator => menu.append(&PredefinedMenuItem::separator(app)?)?,
            Item::Submenu { label, items } => {
                let sub = Submenu::new(app, *label, true)?;
                fill(app, &sub, items, entries)?;
                menu.append(&sub)?;
            }
            Item::Action {
                label,
                accelerator,
                action,
                checked,
            } => {
                let id = format!("m{}", entries.len());
                entries.push(Entry::Core(action.clone()));
                match checked {
                    Some(on) => menu.append(&CheckMenuItem::with_id(
                        app,
                        id,
                        *label,
                        true,
                        *on,
                        *accelerator,
                    )?)?,
                    None => {
                        menu.append(&MenuItem::with_id(app, id, *label, true, *accelerator)?)?
                    }
                }
            }
            Item::Role { label, role } => {
                let id = format!("m{}", entries.len());
                entries.push(Entry::Role(*role));
                menu.append(&MenuItem::with_id(
                    app,
                    id,
                    *label,
                    true,
                    accelerator(*role),
                )?)?;
            }
        }
    }
    Ok(())
}

/// Builds the menu bar for the settings now in force and keeps the entries by id. Called again whenever the
/// settings change, so the speed limit and the end-of-queue choice are ticked.
pub fn rebuild(app: &AppHandle) {
    let app2 = app.clone();
    // Menus are native objects: build them on the thread that owns the window.
    let _ = app.run_on_main_thread(move || {
        let shell = app2.state::<Shell>();
        let s = shell.manager.settings();
        let template =
            spec::menu_template(s.speed_limit_kbps, &s.after_all, cfg!(debug_assertions));
        let mut entries = Vec::new();
        let built = (|| -> tauri::Result<Menu<tauri::Wry>> {
            let bar = Menu::new(&app2)?;
            for m in &template {
                let sub = Submenu::new(&app2, m.label, true)?;
                fill(&app2, &sub, &m.items, &mut entries)?;
                bar.append(&sub)?;
            }
            Ok(bar)
        })();
        match built {
            Ok(bar) => {
                *shell.entries.lock().unwrap_or_else(|e| e.into_inner()) = entries;
                if let Some(w) = main_window(&app2) {
                    let _ = w.set_menu(bar);
                }
            }
            Err(e) => eprintln!("menu: {e}"),
        }
    });
}

/// A menu entry was chosen.
pub fn on_event(app: &AppHandle, id: &str) {
    let Some(index) = id.strip_prefix('m').and_then(|n| n.parse::<usize>().ok()) else {
        return;
    };
    let entry = app
        .state::<Shell>()
        .entries
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(index)
        .cloned();
    match entry {
        Some(Entry::Core(a)) => run(app, a),
        Some(Entry::Role(r)) => role(app, r),
        None => {}
    }
}

/// Real quit: the close button stops hiding the window and the exit goes through the clean shutdown.
pub fn quit(app: &AppHandle) {
    app.state::<Shell>()
        .quitting
        .store(true, std::sync::atomic::Ordering::Relaxed);
    app.exit(0);
}

pub fn pause_all(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let shell = app.state::<Shell>();
        for t in shell.manager.list() {
            if matches!(t.state.status, Status::Downloading | Status::Queued) {
                let _ = shell.manager.pause(&t.state.id).await;
            }
        }
    });
}

pub fn resume_all(app: &AppHandle) {
    let shell = app.state::<Shell>();
    for t in shell.manager.list() {
        if matches!(t.state.status, Status::Paused | Status::Error) {
            let _ = shell.manager.resume(&t.state.id);
        }
    }
}

pub fn open_download_folder(app: &AppHandle) {
    let dir = app.state::<Shell>().manager.settings().dir;
    let _ = app.opener().open_path(dir, None::<&str>);
}

fn run(app: &AppHandle, action: Action) {
    let shell = app.state::<Shell>();
    match action {
        Action::Ui(what, arg) => {
            show_window(app);
            send(app, "dm:menu", json!({ "action": what, "arg": arg }));
        }
        Action::OpenFolder => open_download_folder(app),
        Action::Quit => quit(app),
        Action::PauseAll => pause_all(app),
        Action::ResumeAll => resume_all(app),
        Action::ClearFinished => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                app.state::<Shell>().manager.clear_completed().await;
            });
        }
        Action::SetLimit(kbps) => {
            let _ = shell
                .manager
                .set_settings(&json!({ "speedLimitKBps": kbps }));
        }
        Action::SetAfterAll(value) => {
            let _ = shell.manager.set_settings(&json!({ "afterAll": value }));
        }
        Action::CheckUpdate => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match crate::update::look_for_update(&app).await {
                    Ok(Some(v)) => attention(
                        &app,
                        format!(
                            "La version {v} est disponible : utilisez le bandeau en haut de la fenêtre pour l’installer."
                        ),
                        false,
                    ),
                    Ok(None) => attention(
                        &app,
                        format!(
                            "Tachy Download est à jour (version {}).",
                            app.state::<Shell>().version
                        ),
                        false,
                    ),
                    Err(e) => attention(
                        &app,
                        format!("Impossible de vérifier les mises à jour : {e}"),
                        true,
                    ),
                }
            });
        }
        Action::CopyDiagnostic => match copy_diagnostic(app) {
            Ok(()) => attention(
                app,
                "Le diagnostic est copié : collez-le dans un message.",
                false,
            ),
            Err(e) => attention(app, e, false),
        },
        Action::OpenProject => {
            let _ = app.opener().open_url(PROJECT_URL, None::<&str>);
        }
        Action::About => attention(
            app,
            format!(
                "Tachy Download {}\nGestionnaire de téléchargement pour Windows.\n{PROJECT_URL}",
                shell.version
            ),
            false,
        ),
    }
}

pub fn copy_diagnostic(app: &AppHandle) -> Result<(), String> {
    let shell = app.state::<Shell>();
    let file = shell.data_dir.join("ytdlp-last.log");
    let text = std::fs::read_to_string(&file)
        .map_err(|_| "Aucun diagnostic enregistré pour le moment".to_string())?;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// Browser-like zoom: each step is 20 %, the way the web view of the Electron version zoomed.
const ZOOM_STEP: f64 = 1.2;

fn role(app: &AppHandle, role: Role) {
    let Some(w) = main_window(app) else { return };
    let shell = app.state::<Shell>();
    let mut zoom = shell.zoom.lock().unwrap_or_else(|e| e.into_inner());
    let current = *zoom;
    let mut apply = |z: f64| {
        *zoom = z.clamp(0.25, 5.0);
        let _ = w.set_zoom(*zoom);
    };
    match role {
        Role::ZoomIn => apply(current * ZOOM_STEP),
        Role::ZoomOut => apply(current / ZOOM_STEP),
        Role::ResetZoom => apply(1.0),
        Role::ToggleFullscreen => {
            let full = w.is_fullscreen().unwrap_or(false);
            let _ = w.set_fullscreen(!full);
        }
        Role::ToggleDevTools => {
            #[cfg(debug_assertions)]
            w.open_devtools();
        }
    }
}
