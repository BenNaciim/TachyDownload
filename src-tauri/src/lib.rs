//! Tachy Download: the Tauri shell around `tachy-core`.

mod fake;

pub fn run() {
    // S4 stand-in: the installer runs `--setup-tools` after copying the files. The real core fetches yt-dlp and
    // ffmpeg here (M3); the throwaway version only leaves a marker so the installer proof can check it ran.
    if std::env::args().any(|a| a == "--setup-tools") {
        let _ = std::fs::write(
            std::env::temp_dir().join("tachy-s4-setup-tools.marker"),
            "ran",
        );
        return;
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            fake::dm_list,
            fake::dm_state,
            fake::dm_tools_status,
            fake::dm_file_icon,
            fake::dm_logins_list,
            fake::dm_settings_set,
            fake::dm_copy_token,
            fake::dm_redownload,
        ])
        .setup(|app| {
            use tauri::Manager;
            use tauri::window::{Effect, EffectsBuilder};
            fake::start_ticker(app.handle().clone());
            // S1: the same DWM backdrop as Electron's `backgroundMaterial: 'acrylic'` (build 22523+ uses DWMSBT_TRANSIENTWINDOW).
            if let Some(window) = app.get_webview_window("main") {
                window.set_effects(EffectsBuilder::new().effect(Effect::Acrylic).build())?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tachy Download could not start");
}
