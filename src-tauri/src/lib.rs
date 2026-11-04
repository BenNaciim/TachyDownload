//! Tachy Download: the Tauri shell around `tachy-core`. The core downloads; this part owns the windows, the menu,
//! the notification-area icon, notifications, the commands the page calls and the extension's local server.

mod autostart;
mod commands;
mod confirm;
mod data;
mod events;
mod icons;
mod menu;
mod notify;
mod secrets;
mod shell;
mod tray;
mod update;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::json;
use tachy_core::manager::{Manager, ManagerConfig};
use tachy_core::server::{AddRequest, ServerConfig, listen};
use tachy_core::state::Status;
use tachy_core::tools::Tools;
use tauri::window::{Effect, EffectsBuilder};
use tauri::{AppHandle, Manager as _, RunEvent, WebviewWindowBuilder, WindowEvent};

use shell::{Shell, glass_supported, show_window};

/// Every window of the app shares one web view environment, so they all share these arguments. The flag turns off
/// GPU compositing, which the acrylic backdrop needs to show through (proof S1).
pub const BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-gpu-compositing";

/// The pages of the app, and nothing else: a link that leaves them is never followed.
pub fn is_app_url(url: &tauri::Url) -> bool {
    matches!(url.scheme(), "tauri" | "about") || matches!(url.host_str(), Some("tauri.localhost"))
}

const PORT: u16 = 6800;

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let setup_tools = args.iter().any(|a| a == "--setup-tools");
    let hidden = args.iter().any(|a| a == "--hidden");

    let mut builder = tauri::Builder::default();
    // The installer runs the app once with --setup-tools to fetch the video tools: no window, no lock.
    if !setup_tools {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app);
        }));
    }
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .invoke_handler(tauri::generate_handler![
            commands::dm_list,
            commands::dm_state,
            commands::dm_add,
            commands::dm_open_page,
            commands::dm_file_icon,
            commands::dm_update_check,
            commands::dm_update_install,
            commands::dm_tools_status,
            commands::dm_tools_install,
            commands::dm_pause,
            commands::dm_resume,
            commands::dm_redownload,
            commands::dm_set_limit,
            commands::dm_remove,
            commands::dm_reveal,
            commands::dm_open,
            commands::dm_settings_set,
            commands::dm_pick_dir,
            commands::dm_copy_token,
            commands::dm_copy_diagnostic,
            commands::dm_logins_list,
            commands::dm_logins_save,
            commands::dm_logins_remove,
            commands::confirm_init,
            commands::confirm_category,
            commands::confirm_pick_dir,
            commands::confirm_probe,
            commands::confirm_answer,
        ])
        .on_menu_event(|app, event| menu::on_event(app, event.id().as_ref()))
        .on_window_event(on_window_event)
        .setup(move |app| {
            let handle = app.handle().clone();
            let custom = data::data_dir_override(&args);
            let default_dir = handle.path().data_dir()?.join("Tachy Download");
            let dir = custom.clone().unwrap_or(default_dir.clone());
            if custom.is_none() {
                let old = default_dir.with_file_name("Download Manager");
                if data::migrate_old_data(&old, &dir) {
                    autostart::forget_old_entry();
                }
            }
            if setup_tools {
                setup_video_tools(handle, dir);
                return Ok(());
            }
            start(&handle, dir, hidden)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Tachy Download could not start");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            shut_down_first(app, || api.prevent_exit());
        }
    });
}

/// The window's close button keeps the downloads running in the notification area (unless turned off); a
/// confirmation window that is closed without an answer cancels its request.
fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    let label = window.label();
    match event {
        WindowEvent::CloseRequested { api, .. } if label == "main" => {
            let Some(shell) = window.try_state::<Shell>() else {
                return;
            };
            if !shell.quitting.load(Ordering::Relaxed) && shell.manager.settings().close_to_tray {
                api.prevent_close();
                let _ = window.hide();
            }
        }
        WindowEvent::Destroyed if label.starts_with("confirm-") => {
            confirm::forget(window.app_handle(), label);
        }
        _ => {}
    }
}

/// Before the app ends, the downloads are saved and the server is closed. The exit is held back until then.
fn shut_down_first(app: &AppHandle, hold: impl FnOnce()) {
    let Some(shell) = app.try_state::<Shell>() else {
        return;
    };
    if shell.shut_down.load(Ordering::Relaxed) {
        return;
    }
    hold();
    if shell.shutting.swap(true, Ordering::Relaxed) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let shell = app.state::<Shell>();
        shell.quitting.store(true, Ordering::Relaxed);
        shell.manager.shutdown().await;
        if let Some(api) = shell.api.lock().unwrap_or_else(|e| e.into_inner()).take() {
            api.close();
        }
        shell.shut_down.store(true, Ordering::Relaxed);
        app.exit(0);
    });
}

/// Downloads the video tools (checksum-verified) and exits. Never fails the installer: offline just skips it.
fn setup_video_tools(app: AppHandle, dir: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let tools = Tools::new(dir.join("tools"));
        let work = async {
            for name in ["ytdlp", "deno", "ffmpeg"] {
                if tools.find(name).is_none() {
                    // the settings page still offers the install button
                    let _ = tools.install(name).await;
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(8 * 60), work).await;
        app.exit(0);
    });
}

fn start(app: &AppHandle, dir: PathBuf, hidden: bool) -> Result<(), Box<dyn std::error::Error>> {
    let token = data::load_token(&dir)?;
    let downloads = app
        .path()
        .download_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let tools = Arc::new(Tools::new(dir.join("tools")));
    let finder = tools.clone();
    // Site logins are encrypted with the Windows account and never leave this process but as request headers.
    let logins = Arc::new(tachy_core::logins::Logins::new(
        &dir.join("logins.json"),
        Arc::new(secrets::Dpapi::new(dir.join("Local State"))),
    ));
    let for_sites = logins.clone();
    let manager = tauri::async_runtime::block_on(async {
        let manager = Manager::new(ManagerConfig {
            store_path: Some(dir.join("downloads.json")),
            settings: json!({ "dir": downloads, "categories": true }),
            tools: Some(Arc::new(move |name| finder.find(name))),
            site_auth: Some(Arc::new(move |url| for_sites.headers_for(url))),
            ..ManagerConfig::default()
        });
        manager.load();
        manager
    });
    if manager.settings().dir.is_empty() {
        let _ = manager.set_settings(&json!({ "dir": downloads }));
    }
    let glass = glass_supported();
    app.manage(Shell {
        manager: manager.clone(),
        tools: tools.clone(),
        logins,
        token: token.clone(),
        port: AtomicU16::new(0),
        data_dir: dir,
        version: app.package_info().version.to_string(),
        confirms: Default::default(),
        confirm_seq: AtomicU64::new(0),
        quitting: AtomicBool::new(false),
        shutting: AtomicBool::new(false),
        shut_down: AtomicBool::new(false),
        zoom: std::sync::Mutex::new(1.0),
        icons: Default::default(),
        entries: Default::default(),
        api: Default::default(),
        last_status: Default::default(),
        pending_update: Default::default(),
        glass,
    });

    // The extension's local server: a download it hands over goes through the confirmation window when that is on.
    let (for_server, for_ask) = (manager.clone(), app.clone());
    let served = tauri::async_runtime::block_on(async {
        manager.start();
        listen(
            ServerConfig {
                manager: for_server,
                token: Arc::new(move || token.clone()),
                on_added: None,
                on_request: Some(Arc::new(move |req: AddRequest| {
                    let confirm = for_ask
                        .state::<Shell>()
                        .manager
                        .settings()
                        .confirm_downloads;
                    if confirm {
                        confirm::ask(&for_ask, &req.url, req.opts.clone(), req.size);
                    }
                    confirm
                })),
            },
            PORT,
            10,
        )
        .await
    });
    let shell = app.state::<Shell>();
    match served {
        Ok(api) => {
            shell.set_port(api.port());
            *shell.api.lock().unwrap_or_else(|e| e.into_inner()) = Some(api);
        }
        Err(e) => {
            shell::attention(
                app,
                format!("Le serveur local n’a pas pu démarrer : {e}"),
                true,
            );
        }
    }

    events::spawn_pump(app);
    events::spawn_tools_pump(app);
    update::spawn_auto_check(app);

    let config = app
        .config()
        .app
        .windows
        .first()
        .cloned()
        .expect("the main window is described in tauri.conf.json");
    let window = WebviewWindowBuilder::from_config(app, &config)?
        .visible(!hidden)
        .additional_browser_args(BROWSER_ARGS)
        .on_navigation(is_app_url)
        .build()?;
    if glass {
        // The same acrylic backdrop as Electron's `backgroundMaterial: 'acrylic'`.
        window.set_effects(EffectsBuilder::new().effect(Effect::Acrylic).build())?;
    }
    menu::rebuild(app);
    tray::create(app)?;
    autostart::apply(app, manager.settings().auto_start);
    spawn_timers(app);
    Ok(())
}

/// Keep our yt-dlp fresh (a check shortly after launch, then every few hours) and look for a new version of the app
/// itself (a quiet look shortly after launch, then twice a day).
fn spawn_timers(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let shell = handle.state::<Shell>();
            let busy = {
                let manager = shell.manager.clone();
                move || {
                    manager.list().iter().any(|t| {
                        t.state.kind == tachy_core::state::Kind::Ytdlp
                            && t.state.status == Status::Downloading
                    })
                }
            };
            // offline or rate-limited: try again later
            let _ = shell
                .tools
                .check_ytdlp_update(Duration::from_secs(20 * 3600), busy)
                .await;
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(45)).await;
        loop {
            let auto = handle.state::<Shell>().manager.settings().auto_update;
            if !cfg!(debug_assertions) && auto {
                let _ = update::look_for_update(&handle).await;
            }
            tokio::time::sleep(Duration::from_secs(12 * 3600)).await;
        }
    });
}
