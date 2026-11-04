//! Tachy Download: the Tauri shell around `tachy-core`.

mod fake;

pub fn run() {
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
            fake::start_ticker(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tachy Download could not start");
}
