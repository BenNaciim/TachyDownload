//! What every task shares with the manager: the current settings and HTTP client, the event channel, the
//! "something changed, save soon" signal and the way back to the manager when a task settles.

use crate::net::{Headers, Net};
use crate::rules::{Rule, folder_for, parse_rules};
use crate::settings::Settings;
use crate::state::{Snapshot, TaskState};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use tokio::sync::{Notify, broadcast, mpsc};

/// Locks a mutex whatever happened to the thread that held it last: one panicking download must not take the
/// others down with it.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the manager tells whoever listens (the window, the tray, the tests).
#[derive(Debug, Clone)]
pub enum Event {
    Update(Box<Snapshot>),
    Removed(String),
    Settings(Box<Settings>),
}

/// What a task tells the manager.
#[derive(Debug)]
pub enum Msg {
    /// The transfer is over (finished, failed or stopped): plan a retry, start the next, post-process.
    Settled(String),
}

/// Extra request headers for the site an address belongs to (a saved login), or none.
pub type SiteAuth = Arc<dyn Fn(&str) -> Option<Headers> + Send + Sync>;

pub struct Ctx {
    pub settings: RwLock<Settings>,
    /// The limit in force (KB/s): what tasks that run other programs pass on.
    pub limit_kbps: std::sync::atomic::AtomicU64,
    pub net: RwLock<Net>,
    pub site_auth: Option<SiteAuth>,
    rules: Mutex<(String, Vec<Rule>)>,
    pub events: broadcast::Sender<Event>,
    pub dirty: Notify,
    /// Held while a download picks its file name and reserves it with its .part file, so two downloads of the
    /// same name never pick the same one.
    pub naming: Mutex<()>,
    pub tx: mpsc::UnboundedSender<Msg>,
}

impl Ctx {
    pub fn new(
        settings: Settings,
        net: Net,
        site_auth: Option<SiteAuth>,
        tx: mpsc::UnboundedSender<Msg>,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(4096);
        Arc::new(Self {
            settings: RwLock::new(settings),
            limit_kbps: std::sync::atomic::AtomicU64::new(0),
            net: RwLock::new(net),
            site_auth,
            rules: Mutex::new((String::new(), Vec::new())),
            events,
            dirty: Notify::new(),
            naming: Mutex::new(()),
            tx,
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn net(&self) -> Net {
        self.net.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn site_headers(&self, url: &str) -> Headers {
        self.site_auth
            .as_ref()
            .and_then(|f| f(url))
            .unwrap_or_default()
    }

    /// Where a finished file goes: the filing rules first, then the type folder when that setting is on. A
    /// download with its own folder stays where it is.
    pub fn final_dir(&self, state: &TaskState, name: &str) -> PathBuf {
        let dir = PathBuf::from(&state.dir);
        let s = self.settings();
        if !state.own_folder {
            let mut cache = lock(&self.rules);
            if cache.0 != s.rules {
                *cache = (s.rules.clone(), parse_rules(&s.rules));
            }
            if let Some(ruled) = folder_for(&cache.1, name, &state.url, &state.page_url, &dir) {
                return ruled;
            }
            if s.categories {
                return dir.join(crate::filetypes::category_for(name));
            }
        }
        dir
    }
}
