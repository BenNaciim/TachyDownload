//! Looking for a new version of the app: the check on GitHub, the banner in the window, and the silent installer.
//! The engine part (which release, the SHA-256 check) is `tachy_core::updater`.

use std::time::Duration;

use serde_json::{Value, json};
use tachy_core::updater::{API, UpdateInfo, check_for_update, download_update};
use tauri::{AppHandle, Manager as _};
use tauri_plugin_opener::OpenerExt;

use crate::shell::{Shell, send};

/// What the window is told about a release.
pub fn summary(info: &UpdateInfo) -> Value {
    json!({ "version": info.version, "page": info.page, "canInstall": info.sha256.is_some() })
}

/// Asks GitHub whether a newer release exists and tells the window about it. Returns its version.
pub async fn look_for_update(app: &AppHandle) -> Result<Option<String>, String> {
    let shell = app.state::<Shell>();
    let info = check_for_update(&shell.version, API).await?;
    send(app, "dm:updateAvail", info.as_ref().map(summary));
    let version = info.as_ref().map(|i| i.version.clone());
    *shell
        .pending_update
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = info;
    Ok(version)
}

/// Downloads and verifies the installer, runs it silently and quits so it can replace the app (it starts the new
/// version when it is done). A release whose installer cannot be verified opens its page instead.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let info = app
        .state::<Shell>()
        .pending_update
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or("Aucune mise à jour à installer")?;
    if info.sha256.is_none() {
        // unverifiable: the user does it by hand
        return app
            .opener()
            .open_url(&info.page, None::<&str>)
            .map_err(|e| e.to_string());
    }
    let emitter = app.clone();
    let file = download_update(
        &info,
        &std::env::temp_dir().join("tachy-update"),
        false,
        move |got, total| {
            send(
                &emitter,
                "dm:updateProgress",
                json!({ "got": got, "total": total }),
            )
        },
    )
    .await?;
    let mut cmd = std::process::Command::new(&file);
    cmd.args(["/S", "/UPDATE", "/R"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008 | 0x0800_0000); // DETACHED_PROCESS | CREATE_NO_WINDOW
    }
    cmd.spawn().map_err(|e| e.to_string())?;
    crate::menu::quit(app);
    Ok(())
}

/// A check shortly after the start and then twice a day, in the installed app only (a development build has no
/// release to compare with) and only while the setting is on.
pub fn spawn_auto_check(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(45)).await;
        loop {
            if app.state::<Shell>().manager.settings().auto_update {
                let _ = look_for_update(&app).await;
            }
            tokio::time::sleep(Duration::from_secs(12 * 3600)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_told_whether_the_installer_can_be_verified() {
        let mut info = UpdateInfo {
            version: "2.1.0".into(),
            url: "https://github.com/x/y/a.exe".into(),
            size: 10,
            sha256: Some("a".repeat(64)),
            page: "https://github.com/x/y/releases/tag/v2.1.0".into(),
        };
        assert_eq!(
            summary(&info),
            json!({ "version": "2.1.0", "page": "https://github.com/x/y/releases/tag/v2.1.0", "canInstall": true })
        );
        info.sha256 = None;
        assert_eq!(summary(&info)["canInstall"], json!(false));
    }
}
