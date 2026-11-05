//! Errors of a transfer, with the French sentences the user sees (`friendlyError` and `httpError` in `util.js`
//! and `download.js`).

use std::error::Error as StdError;
use std::fmt;
use std::io;

/// A failed transfer. `message` is already the sentence to show.
#[derive(Debug, Clone, Default)]
pub struct TaskError {
    pub message: String,
    /// A refusal or a bad answer: retrying will not help.
    pub fatal: bool,
    /// 401, 403 or 410: a signed link has probably expired.
    pub expired: bool,
    pub status: Option<u16>,
    /// A fatal error that is nevertheless worth another try later.
    pub retryable: bool,
    /// The remote file changed under a ranged request (a 200 to a `Range`).
    pub changed: bool,
    /// 429 or 503: the server wants fewer connections.
    pub throttled: bool,
    pub retry_after_secs: u64,
}

impl TaskError {
    /// A transient failure (a dropped connection, a 5xx...): the transfer may be retried.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }

    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fatal: true,
            ..Self::default()
        }
    }

    /// A refused request: 4xx other than 429. 401/403/410 usually mean a signed link has expired.
    pub fn http(status: u16) -> Self {
        Self {
            message: format!("Le serveur a répondu {status}"),
            fatal: true,
            expired: matches!(status, 401 | 403 | 410),
            status: Some(status),
            ..Self::default()
        }
    }

    pub fn from_io(err: &io::Error) -> Self {
        Self::new(friendly_io(err))
    }

    pub fn from_reqwest(err: &reqwest::Error) -> Self {
        Self::new(friendly_reqwest(err))
    }
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl StdError for TaskError {}

impl From<io::Error> for TaskError {
    fn from(err: io::Error) -> Self {
        Self::from_io(&err)
    }
}

/// Why a transfer function stopped: the user stopped it, or it failed.
#[derive(Debug, Clone)]
pub enum Fail {
    Cancelled,
    Error(TaskError),
}

impl From<TaskError> for Fail {
    fn from(e: TaskError) -> Self {
        Fail::Error(e)
    }
}

impl From<io::Error> for Fail {
    fn from(e: io::Error) -> Self {
        Fail::Error(TaskError::from_io(&e))
    }
}

pub type Result<T> = std::result::Result<T, Fail>;

pub const MSG_NOT_FOUND: &str = "Adresse introuvable. Vérifiez le lien et votre connexion.";
pub const MSG_REFUSED: &str = "Le serveur refuse la connexion.";
pub const MSG_TIMEOUT: &str = "Le serveur ne répond pas.";
pub const MSG_INTERRUPTED: &str = "La connexion a été interrompue.";
pub const MSG_DISK_FULL: &str = "Le disque est plein.";
pub const MSG_ACCESS: &str = "Accès refusé au dossier de destination.";
pub const MSG_INVALID_URL: &str = "Le lien est invalide.";
pub const MSG_BAD_PORT: &str = "Ce port est bloqué par mesure de sécurité.";
pub const MSG_FETCH_FAILED: &str = "Connexion impossible au serveur.";

/// Windows socket and disk codes that `io::ErrorKind` does not name.
fn friendly_io(err: &io::Error) -> String {
    use io::ErrorKind::*;
    match err.raw_os_error() {
        Some(112) | Some(39) | Some(28) => return MSG_DISK_FULL.into(), // ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL, ENOSPC
        Some(11001) | Some(11002) | Some(11004) => return MSG_NOT_FOUND.into(), // WSAHOST_NOT_FOUND, WSATRY_AGAIN, WSANO_DATA
        _ => {}
    }
    match err.kind() {
        ConnectionRefused => MSG_REFUSED.into(),
        TimedOut => MSG_TIMEOUT.into(),
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof => MSG_INTERRUPTED.into(),
        PermissionDenied => MSG_ACCESS.into(),
        StorageFull => MSG_DISK_FULL.into(),
        _ => err.to_string(),
    }
}

fn friendly_reqwest(err: &reqwest::Error) -> String {
    // The io error that caused it, wherever it sits in the chain.
    let mut cur: Option<&(dyn StdError + 'static)> = err.source();
    let mut text = String::new();
    while let Some(e) = cur {
        if let Some(io) = e.downcast_ref::<io::Error>() {
            let msg = friendly_io(io);
            if msg != io.to_string() {
                return msg;
            }
        }
        text.push_str(&e.to_string());
        text.push(' ');
        cur = e.source();
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("dns error")
        || lower.contains("failed to lookup")
        || lower.contains("no such host")
    {
        return MSG_NOT_FOUND.into();
    }
    if err.is_timeout() {
        return MSG_TIMEOUT.into();
    }
    if err.is_builder() {
        return MSG_INVALID_URL.into();
    }
    if err.is_body()
        || err.is_decode()
        || lower.contains("connection closed")
        || lower.contains("connection reset")
    {
        return MSG_INTERRUPTED.into();
    }
    MSG_FETCH_FAILED.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_errors_flag_expired_links() {
        assert!(TaskError::http(403).expired);
        assert!(TaskError::http(401).expired);
        assert!(TaskError::http(410).expired);
        assert!(!TaskError::http(404).expired);
        assert!(TaskError::http(404).fatal);
        assert_eq!(TaskError::http(404).message, "Le serveur a répondu 404");
    }

    #[test]
    fn io_errors_become_sentences() {
        assert_eq!(
            TaskError::from_io(&io::Error::from(io::ErrorKind::ConnectionRefused)).message,
            MSG_REFUSED
        );
        assert_eq!(
            TaskError::from_io(&io::Error::from(io::ErrorKind::PermissionDenied)).message,
            MSG_ACCESS
        );
        assert_eq!(
            TaskError::from_io(&io::Error::from_raw_os_error(112)).message,
            MSG_DISK_FULL
        );
        assert!(!TaskError::from_io(&io::Error::from(io::ErrorKind::ConnectionReset)).fatal);
    }
}
