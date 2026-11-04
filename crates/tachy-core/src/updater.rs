//! Looking for a newer release of the app on GitHub and fetching its installer (`updater.js`). The installer is only
//! kept when its SHA-256 is the one GitHub published for that file.

use futures_util::StreamExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "BenNaciim/TachyDownload";
pub const API: &str = "https://api.github.com";
const AGENT: &str = "TachyDownload-Updater";

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
    pub size: u64,
    /// The checksum GitHub published for the installer, when it did.
    pub sha256: Option<String>,
    /// The release page, where the user can go when the installer cannot be verified.
    pub page: String,
}

/// Compares two versions numerically, ignoring a leading "v" and anything after a "-" ("2.0.0-beta" is "2.0.0").
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> Vec<u64> {
        let v = v.trim_start_matches(['v', 'V']);
        v.split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|n| {
                let digits: String = n.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().unwrap_or(0)
            })
            .collect()
    }
    let (x, y) = (parts(a), parts(b));
    for i in 0..x.len().max(y.len()) {
        let o = x.get(i).unwrap_or(&0).cmp(y.get(i).unwrap_or(&0));
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

fn is_installer(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("tachy-download-setup-")
        && n.ends_with(".exe")
        && n.len() > "tachy-download-setup-.exe".len()
}

/// Looks for a newer release. `None` when there is none, when the repository has no public release (a private
/// repository answers 404), or when the release carries no installer.
pub async fn check_for_update(current: &str, api: &str) -> Result<Option<UpdateInfo>, String> {
    let client = reqwest::Client::builder()
        .user_agent(AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
        .get(format!("{api}/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(format!("GitHub a répondu {}", status.as_u16()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    let rel: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let flag = |k: &str| rel[k].as_bool().unwrap_or(false);
    let Some(tag) = rel["tag_name"].as_str().filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if flag("draft") || flag("prerelease") || compare_versions(tag, current) != Ordering::Greater {
        return Ok(None);
    }
    let asset = rel["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str().is_some_and(is_installer));
    let Some(asset) = asset else {
        return Ok(None);
    };
    let Some(url) = asset["browser_download_url"]
        .as_str()
        .filter(|u| !u.is_empty())
    else {
        return Ok(None);
    };
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|d| {
            d.strip_prefix("sha256:")
                .or_else(|| d.strip_prefix("SHA256:"))
        })
        .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase);
    Ok(Some(UpdateInfo {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: url.to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
        sha256,
        page: rel["html_url"]
            .as_str()
            .filter(|p| !p.is_empty())
            .map_or_else(
                || format!("https://github.com/{REPO}/releases"),
                str::to_string,
            ),
    }))
}

fn allowed(url: &str, loopback: bool) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    if loopback && host == "127.0.0.1" {
        return true; // the tests' stand-in for GitHub
    }
    u.scheme() == "https"
        && (host == "github.com"
            || host.ends_with(".github.com")
            || host == "githubusercontent.com"
            || host.ends_with(".githubusercontent.com"))
}

/// Downloads the installer into `dir` and checks its SHA-256 against the one GitHub published. Without that checksum
/// nothing is run: the caller can still send the user to the release page. `loopback` lets the tests use a local
/// server. `on_progress(got, total)` is called as the bytes arrive. Returns the file.
pub async fn download_update(
    info: &UpdateInfo,
    dir: &Path,
    loopback: bool,
    on_progress: impl Fn(u64, u64),
) -> Result<PathBuf, String> {
    if !allowed(&info.url, loopback) {
        return Err("Adresse de mise à jour refusée".into());
    }
    let Some(expected) = info.sha256.as_deref() else {
        return Err("Somme de contrôle absente : installation automatique refusée".into());
    };
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let safe: String = info
        .version
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        .collect();
    let file = dir.join(format!("Tachy-Download-Setup-{safe}.exe"));
    let part = dir.join(format!("Tachy-Download-Setup-{safe}.exe.part"));

    let result = async {
        let client = reqwest::Client::builder()
            .user_agent(AGENT)
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        let res = client
            .get(&info.url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!(
                "Téléchargement impossible ({})",
                res.status().as_u16()
            ));
        }
        let total = res.content_length().filter(|n| *n > 0).unwrap_or(info.size);
        let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut got = 0u64;
        let mut body = res.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            hash.update(&chunk);
            out.write_all(&chunk).map_err(|e| e.to_string())?;
            got += chunk.len() as u64;
            on_progress(got, total);
        }
        out.flush().map_err(|e| e.to_string())?;
        drop(out);
        let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if digest != expected {
            return Err("Somme de contrôle incorrecte : fichier rejeté".into());
        }
        std::fs::rename(&part, &file).map_err(|e| e.to_string())
    }
    .await;
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    Ok(file)
}
