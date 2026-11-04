//! Looking for a new version of the app. Milestone 5 brings the real check (GitHub releases, SHA-256 digest, silent
//! installer); until then there is never an update to offer.

use tauri::AppHandle;

/// Asks whether a newer release exists and tells the window about it. Returns its version.
pub async fn look_for_update(_app: &AppHandle) -> Result<Option<String>, String> {
    Ok(None)
}
