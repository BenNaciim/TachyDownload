//! Logins kept per site (`logins.js`). Secrets are stored encrypted (`Crypt` is the Windows account's DPAPI in the
//! app) and never leave this module except as request headers for that site.

use crate::ctx::lock;
use crate::net::Headers;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const KINDS: [&str; 3] = ["basic", "bearer", "cookie"];

/// What encrypts the secrets. Errors are the text shown to the user.
pub trait Crypt: Send + Sync {
    fn encrypt(&self, text: &str) -> Result<String, String>;
    fn decrypt(&self, blob: &str) -> Result<String, String>;
    /// A secret saved in the format of an older app: it is decrypted and saved again in the current format.
    fn is_legacy(&self, _blob: &str) -> bool {
        false
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    host: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    user: String,
    secret: String,
}

/// What the window may see of an entry: never the secret.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Listed {
    pub host: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub user: String,
}

/// The host out of whatever the user typed: "example.com", "https://example.com/page", "Example.com:8080".
pub fn clean_host(value: &str) -> Result<String, String> {
    static SCHEME_STAR: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^((?:[a-z][a-z0-9+.-]*://)?)\*\.").unwrap()
    });
    static SCHEME: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^[a-z][a-z0-9+.-]*://").unwrap());
    let trimmed = value.trim();
    // "*.example.com" means the site and its subdomains
    let raw = SCHEME_STAR.replace(trimmed, "$1");
    if raw.is_empty() {
        return Err("Indiquez le site".into());
    }
    let with_scheme = if SCHEME.is_match(&raw) {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let invalid = || "Adresse du site invalide".to_string();
    let host = url::Url::parse(&with_scheme)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .ok_or_else(invalid)?;
    let plain = host
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    if host.is_empty() || (!plain && !host.contains(':')) {
        return Err(invalid());
    }
    let host = host.strip_prefix('*').unwrap_or(&host);
    Ok(host.strip_prefix('.').unwrap_or(host).to_string())
}

fn on_site(host: &str, site: &str) -> bool {
    host == site || host.ends_with(&format!(".{site}"))
}

pub struct Logins {
    file: PathBuf,
    crypt: Arc<dyn Crypt>,
    entries: Mutex<Vec<Entry>>,
}

impl Logins {
    /// Reads the file (a missing or damaged one starts empty) and brings over what an older app saved: the old file is
    /// first copied to `<file>.electron.bak`, once.
    pub fn new(file: &Path, crypt: Arc<dyn Crypt>) -> Logins {
        let entries: Vec<Entry> = std::fs::read(file)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<serde_json::Value>>(&b).ok())
            .map(|v| {
                v.into_iter()
                    .filter_map(|e| serde_json::from_value::<Entry>(e).ok())
                    .filter(|e| KINDS.contains(&e.kind.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let l = Logins {
            file: file.to_path_buf(),
            crypt,
            entries: Mutex::new(entries),
        };
        l.bring_over();
        l
    }

    fn bring_over(&self) {
        let mut entries = lock(&self.entries);
        if !entries.iter().any(|e| self.crypt.is_legacy(&e.secret)) {
            return;
        }
        let mut backup = self.file.clone().into_os_string();
        backup.push(".electron.bak");
        let backup = PathBuf::from(backup);
        if !backup.exists() {
            let _ = std::fs::copy(&self.file, &backup);
        }
        for e in entries
            .iter_mut()
            .filter(|e| self.crypt.is_legacy(&e.secret))
        {
            // one that cannot be read stays as it is: it is listed, and can be removed
            if let Ok(clear) = self.crypt.decrypt(&e.secret)
                && let Ok(again) = self.crypt.encrypt(&clear)
            {
                e.secret = again;
            }
        }
        let _ = self.persist(&entries);
    }

    fn persist(&self, entries: &[Entry]) -> Result<(), String> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut tmp = self.file.clone().into_os_string();
        tmp.push(".tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec(entries).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.file).map_err(|e| e.to_string())
    }

    fn listed(entries: &[Entry]) -> Vec<Listed> {
        let mut v: Vec<Listed> = entries
            .iter()
            .map(|e| Listed {
                host: e.host.clone(),
                kind: e.kind.clone(),
                user: e.user.clone(),
            })
            .collect();
        v.sort_by(|a, b| a.host.cmp(&b.host));
        v
    }

    pub fn list(&self) -> Vec<Listed> {
        Self::listed(&lock(&self.entries))
    }

    pub fn save(
        &self,
        host: &str,
        kind: &str,
        user: &str,
        secret: &str,
    ) -> Result<Vec<Listed>, String> {
        if !KINDS.contains(&kind) {
            return Err("Type de connexion inconnu".into());
        }
        let host = clean_host(host)?;
        let user = user.trim();
        if secret.is_empty() {
            return Err(match kind {
                "cookie" => "Collez le cookie",
                "bearer" => "Collez le jeton",
                _ => "Indiquez le mot de passe",
            }
            .into());
        }
        if kind == "basic" && user.is_empty() {
            return Err("Indiquez le nom d’utilisateur".into());
        }
        if user.chars().count() > 256 || secret.chars().count() > 4096 {
            return Err("Valeur trop longue".into());
        }
        let secret = self.crypt.encrypt(secret)?;
        let mut entries = lock(&self.entries);
        entries.retain(|e| e.host != host);
        entries.push(Entry {
            host,
            kind: kind.to_string(),
            user: if kind == "basic" {
                user.to_string()
            } else {
                String::new()
            },
            secret,
        });
        self.persist(&entries)?;
        Ok(Self::listed(&entries))
    }

    pub fn remove(&self, host: &str) -> Result<Vec<Listed>, String> {
        let host = clean_host(host)?;
        let mut entries = lock(&self.entries);
        entries.retain(|e| e.host != host);
        self.persist(&entries)?;
        Ok(Self::listed(&entries))
    }

    /// Request headers for this address, or none when no login is kept for its site.
    pub fn headers_for(&self, url: &str) -> Option<Headers> {
        let host = url::Url::parse(url).ok()?.host_str()?.to_lowercase();
        // The most specific site wins: login.example.com before example.com.
        let e = lock(&self.entries)
            .iter()
            .filter(|x| on_site(&host, &x.host))
            .max_by_key(|x| x.host.len())
            .cloned()?;
        // saved under another Windows account: unreadable here
        let secret = self.crypt.decrypt(&e.secret).ok()?;
        Some(vec![match e.kind.as_str() {
            "basic" => (
                "Authorization".into(),
                format!("Basic {}", STANDARD.encode(format!("{}:{secret}", e.user))),
            ),
            "bearer" => ("Authorization".into(), format!("Bearer {secret}")),
            _ => ("Cookie".into(), secret),
        }])
    }
}
