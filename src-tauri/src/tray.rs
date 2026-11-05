//! The notification-area icon and the taskbar progress (`createTray` in `main.js`): a menu, a tooltip that says what
//! is running, and the overall progress on the taskbar button, like a browser download does.

use std::time::Duration;

use tachy_core::progress::{Mode, overall_progress};
use tachy_core::state::Status;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Manager as _};

use crate::menu::{open_download_folder, pause_all, quit, resume_all};
use crate::shell::{Shell, main_window, show_window};

const ID: &str = "tachy";

/// "Tachy Download : 2 en cours (43 %), 12,5 Mo/s", or just the name when nothing runs.
pub fn tooltip(running: usize, percent: Option<u32>, bytes_per_second: f64) -> String {
    if running == 0 {
        return "Tachy Download".into();
    }
    let pct = percent.map(|p| format!(" ({p} %)")).unwrap_or_default();
    let speed = format!("{:.1}", bytes_per_second / 1_048_576.0).replace('.', ",");
    format!("Tachy Download : {running} en cours{pct}, {speed} Mo/s")
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let item = |id: &str, label: &str| MenuItem::with_id(app, id, label, true, None::<&str>);
    let menu = Menu::with_items(
        app,
        &[
            &item("open", "Ouvrir Tachy Download")?,
            &item("folder", "Ouvrir le dossier de téléchargement")?,
            &PredefinedMenuItem::separator(app)?,
            &item("pause", "Tout mettre en pause")?,
            &item("resume", "Tout reprendre")?,
            &PredefinedMenuItem::separator(app)?,
            &item("quit", "Quitter")?,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id(ID)
        .tooltip("Tachy Download")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "folder" => open_download_folder(app),
            "pause" => pause_all(app),
            "resume" => resume_all(app),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut last = (String::new(), None::<(Mode, u64)>);
        loop {
            tick.tick().await;
            refresh(&app, &mut last);
        }
    });
    Ok(())
}

/// Updates the tooltip and the taskbar button, only when something changed.
fn refresh(app: &AppHandle, last: &mut (String, Option<(Mode, u64)>)) {
    let all = app.state::<Shell>().manager.list();
    let running: Vec<_> = all
        .iter()
        .filter(|t| t.state.status == Status::Downloading)
        .collect();
    let speed: f64 = running.iter().map(|t| t.speed).sum();
    let progress = overall_progress(&all);
    let percent = (progress.mode == Mode::Normal).then(|| (progress.value * 100.0).floor() as u32);
    let text = tooltip(running.len(), percent, speed);
    if text != last.0 {
        if let Some(tray) = app.tray_by_id(ID) {
            let _ = tray.set_tooltip(Some(&text));
        }
        last.0 = text;
    }
    let bar = (
        progress.mode,
        (progress.value.max(0.0) * 100.0).floor() as u64,
    );
    if last.1 != Some(bar) {
        if let Some(w) = main_window(app) {
            let (status, value) = match progress.mode {
                Mode::None => (ProgressBarStatus::None, None),
                Mode::Indeterminate => (ProgressBarStatus::Indeterminate, None),
                Mode::Normal => (ProgressBarStatus::Normal, Some(bar.1)),
            };
            let _ = w.set_progress_bar(ProgressBarState {
                status: Some(status),
                progress: value,
            });
        }
        last.1 = Some(bar);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tooltip_names_what_runs_with_its_progress_and_speed() {
        assert_eq!(tooltip(0, None, 0.0), "Tachy Download");
        assert_eq!(
            tooltip(2, Some(43), 12.5 * 1_048_576.0),
            "Tachy Download : 2 en cours (43 %), 12,5 Mo/s"
        );
        assert_eq!(
            tooltip(1, None, 0.0),
            "Tachy Download : 1 en cours, 0,0 Mo/s",
            "no size known: no percentage"
        );
    }
}
