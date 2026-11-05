//! Desktop notifications when a download ends while the window is not in front (`onStatusChange` in `main.js`).

use std::path::PathBuf;

use tachy_core::state::{Snapshot, Status};
use tauri::AppHandle;

#[cfg(windows)]
pub fn toast(app: &AppHandle, snap: &Snapshot) {
    use tauri_plugin_opener::OpenerExt;
    use tauri_winrt_notification::Toast;

    let completed = snap.state.status == Status::Completed;
    let body = if completed {
        snap.state.filename.clone()
    } else {
        format!(
            "{} : {}",
            snap.state.filename,
            snap.state.error.as_deref().unwrap_or("")
        )
    };
    // An installed app has its own identity (the shortcut the installer makes); a development run borrows
    // PowerShell's, which is the only one Windows accepts for an unregistered program.
    let identity = if cfg!(debug_assertions) {
        Toast::POWERSHELL_APP_ID.to_string()
    } else {
        app.config().identifier.clone()
    };
    let file = PathBuf::from(&snap.state.dir).join(&snap.state.filename);
    let handle = app.clone();
    let shown = Toast::new(&identity)
        .title(if completed {
            "Téléchargement terminé"
        } else {
            "Téléchargement échoué"
        })
        .text1(&body)
        .on_activated(move |_| {
            if completed {
                let _ = handle.opener().reveal_item_in_dir(&file);
            } else {
                crate::shell::show_window(&handle);
            }
            Ok(())
        })
        .show();
    if let Err(e) = shown {
        eprintln!("notification: {e}");
    }
}

#[cfg(not(windows))]
pub fn toast(_app: &AppHandle, _snap: &Snapshot) {}
