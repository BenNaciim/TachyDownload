//! Starting with Windows (`applyAutoStart` in `main.js`): the app is started hidden, in the notification area.

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

pub fn apply(app: &AppHandle, enabled: bool) {
    // A development run must not register the debug build as a startup program.
    if cfg!(debug_assertions) {
        return;
    }
    let launcher = app.autolaunch();
    let _ = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
}

/// The startup entry of the old "Download Manager" name, left behind by the previous app.
pub fn forget_old_entry() {
    if cfg!(debug_assertions) {
        return;
    }
    crate::events::run_hidden(
        "reg",
        &[
            "delete",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "Download Manager",
            "/f",
        ],
    );
}
