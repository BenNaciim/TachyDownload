//! What the shell keeps while the app runs, and the small helpers every part of it shares.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};

use serde::Serialize;
use tachy_core::manager::Manager;
use tachy_core::server::{AddRequest, ExtensionServer};
use tachy_core::state::Status;
use tachy_core::tools::Tools;
use tauri::{AppHandle, Emitter, Manager as _, WebviewWindow};

use crate::icons::IconCache;
use crate::menu::Entry;

pub const PROJECT_URL: &str = "https://github.com/BenNaciim/TachyDownload";

/// A "Download this?" window that is open.
pub struct Confirm {
    pub req: AddRequest,
    pub initial_name: String,
    pub done: bool,
}

pub struct Shell {
    pub manager: Manager,
    pub tools: std::sync::Arc<Tools>,
    pub logins: std::sync::Arc<tachy_core::logins::Logins>,
    pub token: String,
    pub port: AtomicU16,
    pub data_dir: PathBuf,
    pub version: String,
    pub confirms: Mutex<HashMap<String, Confirm>>,
    pub confirm_seq: AtomicU64,
    /// Set when the user really quits: the close button then no longer hides the window.
    pub quitting: AtomicBool,
    /// Set when the shutdown has begun, so it runs only once.
    pub shutting: AtomicBool,
    /// Set once the downloads are saved and the server is closed: the exit goes through.
    pub shut_down: AtomicBool,
    pub zoom: Mutex<f64>,
    pub icons: IconCache,
    pub entries: Mutex<Vec<Entry>>,
    pub api: Mutex<Option<ExtensionServer>>,
    pub last_status: Mutex<HashMap<String, Status>>,
    pub pending_update: Mutex<Option<tachy_core::updater::UpdateInfo>>,
    pub glass: bool,
}

impl Shell {
    pub fn set_port(&self, port: u16) {
        self.port.store(port, Ordering::Relaxed);
    }
}

/// Tells the main window something (`dm:update`, `dm:menu`...).
pub fn send<P: Serialize + Clone>(app: &AppHandle, event: &str, payload: P) {
    let _ = app.emit_to("main", event, payload);
}

pub fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

pub fn show_window(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Where the file of a download is: the finished one, else the `.part` being written.
pub fn task_path(shell: &Shell, id: &str) -> Result<PathBuf, String> {
    let t = shell
        .manager
        .list()
        .into_iter()
        .find(|t| t.state.id == id)
        .ok_or("Téléchargement introuvable")?;
    let finished = PathBuf::from(&t.state.dir).join(&t.state.filename);
    Ok(if finished.exists() {
        finished
    } else {
        PathBuf::from(&t.state.dir).join(format!("{}.part", t.state.filename))
    })
}

/// Acrylic backgrounds need Windows 11 22H2 (build 22621) or later.
pub fn glass_supported() -> bool {
    windows_build() >= 22621
}

/// The Windows 11 acrylic backdrop, as the system material of the window (DWMWA_SYSTEMBACKDROP_TYPE = transient
/// window), the way Electron's `backgroundMaterial: 'acrylic'` does it. Tauri's own `Effect::Acrylic` uses the older
/// per-window accent, which with software compositing keeps the GPU process busy (about 9 % of a core at rest).
#[cfg(windows)]
pub fn apply_glass(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows::Win32::Graphics::Dwm::{
        DWM_SYSTEMBACKDROP_TYPE, DWMWA_SYSTEMBACKDROP_TYPE, DwmSetWindowAttribute,
    };
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let transient_window = DWM_SYSTEMBACKDROP_TYPE(3);
    // SAFETY: `hwnd` is this window's handle; the value is a 4-byte DWM_SYSTEMBACKDROP_TYPE, the size passed.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &transient_window as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    }
    .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn apply_glass(_: &tauri::WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn windows_build() -> u32 {
    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        csd: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
    }
    let mut info = OsVersionInfo {
        size: std::mem::size_of::<OsVersionInfo>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        csd: [0; 128],
    };
    // SAFETY: `info` is a valid OSVERSIONINFOW of the declared size, as RtlGetVersion requires.
    if unsafe { RtlGetVersion(&mut info) } == 0 && info.major == 10 {
        info.build
    } else {
        0
    }
}

#[cfg(not(windows))]
fn windows_build() -> u32 {
    0
}

/// A message box without a way out but OK.
pub fn attention(app: &AppHandle, message: impl Into<String>, error: bool) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    app.dialog()
        .message(message.into())
        .title("Tachy Download")
        .kind(if error {
            MessageDialogKind::Error
        } else {
            MessageDialogKind::Info
        })
        .show(|_| {});
}
