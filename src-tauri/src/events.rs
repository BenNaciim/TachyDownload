//! What the engine tells the shell: updates go to the window, a finished download may raise a notification, and when
//! the queue is empty the end-of-queue action runs.

use std::process::Command;

use tachy_core::afterall::{command_for, is_idle};
use tachy_core::ctx::Event;
use tachy_core::state::{Snapshot, Status};
use tauri::{AppHandle, Manager as _};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::shell::{Shell, main_window, send};

/// Forwards the manager's events to the window until the app ends.
pub fn spawn_pump(app: &AppHandle) {
    let app = app.clone();
    let mut rx = app.state::<Shell>().manager.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(Event::Update(snap)) => {
                    send(&app, "dm:update", &*snap);
                    on_status_change(&app, &snap);
                }
                Ok(Event::Removed(id)) => send(&app, "dm:removed", id),
                Ok(Event::Settings(s)) => {
                    send(&app, "dm:settings", &*s);
                    // the limit and end-of-queue choices are ticked in the menu
                    crate::menu::rebuild(&app);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Forwards the progress of the video tools' installation to the window.
pub fn spawn_tools_pump(app: &AppHandle) {
    let app = app.clone();
    let mut rx = app.state::<Shell>().tools.subscribe();
    tauri::async_runtime::spawn(async move {
        while let Ok(status) = rx.recv().await {
            send(&app, "dm:tools", &status);
        }
    });
}

/// A download just finished or failed: notify when the window is out of sight, then maybe run the
/// end-of-queue action.
fn on_status_change(app: &AppHandle, snap: &Snapshot) {
    let shell = app.state::<Shell>();
    let prev = shell
        .last_status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(snap.state.id.clone(), snap.state.status);
    let now = snap.state.status;
    let Some(prev) = prev else { return };
    if prev == now || !matches!(now, Status::Completed | Status::Error) {
        return;
    }
    let settings = shell.manager.settings();
    let visible = main_window(app)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false));
    if settings.notifications && !visible {
        crate::notify::toast(app, snap);
    }
    if settings.after_all != "none" && is_idle(&shell.manager.list()) {
        run_after_all(app, &settings.after_all);
    }
}

/// The end-of-queue action runs once: it is switched off first so it never fires twice by surprise.
fn run_after_all(app: &AppHandle, action: &str) {
    let shell = app.state::<Shell>();
    let _ = shell
        .manager
        .set_settings(&serde_json::json!({ "afterAll": "none" }));
    shell.manager.save_now();
    if action == "quit" {
        crate::menu::quit(app);
        return;
    }
    let Some(cmd) = command_for(action) else {
        return;
    };
    run_hidden(cmd.cmd, &cmd.args);
    if action == "shutdown" {
        app.dialog()
            .message("Les téléchargements sont terminés : arrêt de l’ordinateur dans 60 secondes.")
            .title("Tachy Download")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Annuler l’arrêt".into(),
                "OK".into(),
            ))
            .show(move |cancel_it| {
                if cancel_it {
                    run_hidden("shutdown", &["/a"]);
                }
            });
    }
}

/// Runs a system command without a console window.
pub fn run_hidden(cmd: &str, args: &[&str]) {
    let mut c = Command::new(cmd);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let _ = c.spawn();
}
