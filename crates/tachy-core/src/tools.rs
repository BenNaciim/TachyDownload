//! The video tools (`tools.js`): yt-dlp, ffmpeg and deno. Where they are, installing them from their official release
//! (every download is checked against the published SHA-256 list before it is kept) and keeping yt-dlp up to date.

use regex::{Regex, RegexBuilder};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;

pub type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Where a tool comes from.
#[derive(Debug, Clone)]
pub struct Source {
    pub file: String,
    pub url: String,
    pub sums: String,
    /// The name the checksum list gives the download (lines end with it).
    pub sum_name: Option<String>,
    /// A pattern whose first group is the hash, when the list is not the usual `hash  name` lines.
    pub sums_re: Option<String>,
    pub zip: bool,
}

/// The official release assets of the three tools.
pub fn official_sources() -> BTreeMap<String, Source> {
    let s =
        |file: &str, url: &str, sums: &str, sum_name: Option<&str>, sums_re: Option<&str>, zip| {
            Source {
                file: file.into(),
                url: url.into(),
                sums: sums.into(),
                sum_name: sum_name.map(str::to_string),
                sums_re: sums_re.map(str::to_string),
                zip,
            }
        };
    BTreeMap::from([
        (
            "ytdlp".to_string(),
            s(
                "yt-dlp.exe",
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe",
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS",
                Some("yt-dlp.exe"),
                None,
                false,
            ),
        ),
        (
            "ffmpeg".to_string(),
            s(
                "ffmpeg.exe",
                "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip",
                "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/checksums.sha256",
                Some("ffmpeg-master-latest-win64-gpl.zip"),
                None,
                true,
            ),
        ),
        // JavaScript engine that yt-dlp needs to solve YouTube's stream-address challenge. Its .sha256sum file is a
        // PowerShell Get-FileHash printout.
        (
            "deno".to_string(),
            s(
                "deno.exe",
                "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip",
                "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip.sha256sum",
                None,
                Some(r"Hash\s*:\s*([0-9a-f]{64})"),
                true,
            ),
        ),
    ])
}

/// An installation in progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Installing {
    pub name: String,
    pub received: u64,
    pub total: u64,
}

/// What the settings window shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolsStatus {
    pub ytdlp: Option<String>,
    pub ffmpeg: Option<String>,
    pub deno: Option<String>,
    pub installing: Option<Installing>,
}

/// How an update check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Update {
    /// There is no copy of our own (a copy found on PATH is never touched).
    Absent,
    /// Checked a short while ago.
    Recent,
    /// A download is running or an installation is in progress.
    Busy,
    Current,
    Updated,
}

type Latest = Arc<dyn Fn() -> BoxFut<Result<String, String>> + Send + Sync>;
type Version = Arc<dyn Fn(PathBuf) -> BoxFut<String> + Send + Sync>;
type Installer = Arc<dyn Fn(String) -> BoxFut<Result<(), String>> + Send + Sync>;

pub struct Tools {
    dir: PathBuf,
    sources: BTreeMap<String, Source>,
    installing: Mutex<Option<Installing>>,
    events: broadcast::Sender<ToolsStatus>,
    latest: Option<Latest>,
    version: Option<Version>,
    installer: Option<Installer>,
}

fn on_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(exe))
        .find(|p| !p.as_os_str().is_empty() && p.is_file())
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(r) = find_file(&p, name) {
                return Some(r);
            }
        } else if e.file_name().to_string_lossy().to_lowercase() == name {
            return Some(p);
        }
    }
    None
}

impl Tools {
    /// `dir` is where our own copies live (the data folder's `tools`).
    pub fn new(dir: impl Into<PathBuf>) -> Tools {
        let (events, _) = broadcast::channel(256);
        Tools {
            dir: dir.into(),
            sources: official_sources(),
            installing: Mutex::new(None),
            events,
            latest: None,
            version: None,
            installer: None,
        }
    }

    /// Other places to download from (the tests use a local server).
    pub fn with_sources(mut self, sources: BTreeMap<String, Source>) -> Tools {
        self.sources = sources;
        self
    }

    /// Replaces the lookup of yt-dlp's latest version (a seam for the tests).
    pub fn with_latest(
        mut self,
        f: impl Fn() -> BoxFut<Result<String, String>> + Send + Sync + 'static,
    ) -> Tools {
        self.latest = Some(Arc::new(f));
        self
    }

    /// Replaces the way the installed version is read (a seam for the tests).
    pub fn with_installed_version(
        mut self,
        f: impl Fn(PathBuf) -> BoxFut<String> + Send + Sync + 'static,
    ) -> Tools {
        self.version = Some(Arc::new(f));
        self
    }

    /// Replaces the installation itself (a seam for the tests).
    pub fn with_installer(
        mut self,
        f: impl Fn(String) -> BoxFut<Result<(), String>> + Send + Sync + 'static,
    ) -> Tools {
        self.installer = Some(Arc::new(f));
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ToolsStatus> {
        self.events.subscribe()
    }

    /// Path of an installed tool: our own copy first, then whatever is on PATH.
    pub fn find(&self, name: &str) -> Option<PathBuf> {
        let src = self.sources.get(name)?;
        let own = self.dir.join(&src.file);
        if own.exists() {
            return Some(own);
        }
        on_path(&src.file)
    }

    pub fn status(&self) -> ToolsStatus {
        let path = |n: &str| self.find(n).map(|p| p.to_string_lossy().into_owned());
        ToolsStatus {
            ytdlp: path("ytdlp"),
            ffmpeg: path("ffmpeg"),
            deno: path("deno"),
            installing: self
                .installing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        }
    }

    fn is_installing(&self) -> bool {
        self.installing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    fn progress(&self, name: &str, received: u64, total: u64) {
        *self.installing.lock().unwrap_or_else(|e| e.into_inner()) = Some(Installing {
            name: name.into(),
            received,
            total,
        });
        let _ = self.events.send(self.status());
    }

    async fn latest_ytdlp(&self) -> Result<String, String> {
        if let Some(f) = &self.latest {
            return f().await;
        }
        let client = reqwest::Client::builder()
            .user_agent("tachy-download")
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| e.to_string())?;
        let res = client
            .get("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("HTTP {}", res.status().as_u16()));
        }
        let text = res.text().await.map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok(v["tag_name"].as_str().unwrap_or("").trim().to_string())
    }

    async fn installed_version(&self, file: PathBuf) -> String {
        if let Some(f) = &self.version {
            return f(file).await;
        }
        let mut cmd = tokio::process::Command::new(file);
        cmd.arg("--version")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        match tokio::time::timeout(Duration::from_secs(15), cmd.output()).await {
            Ok(Ok(o)) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            }
            _ => String::new(),
        }
    }

    /// YouTube changes often and an old yt-dlp stops working, so our own copy is refreshed from the official release
    /// (checksum-verified) at most once per `every`. A copy found on PATH is never touched. A failed lookup is an
    /// error and leaves the copy as it is.
    pub async fn check_ytdlp_update(
        &self,
        every: Duration,
        busy: impl Fn() -> bool,
    ) -> Result<Update, String> {
        let Some(src) = self.sources.get("ytdlp") else {
            return Ok(Update::Absent);
        };
        let own = self.dir.join(&src.file);
        if !own.exists() {
            return Ok(Update::Absent);
        }
        let stamp = self.dir.join("ytdlp.checked");
        if let Ok(age) = std::fs::metadata(&stamp)
            .and_then(|m| m.modified())
            .and_then(|t| {
                SystemTime::now()
                    .duration_since(t)
                    .map_err(std::io::Error::other)
            })
            && age < every
        {
            return Ok(Update::Recent);
        }
        if self.is_installing() || busy() {
            return Ok(Update::Busy);
        }
        let latest = self.latest_ytdlp().await?;
        if latest.is_empty() {
            return Ok(Update::Current);
        }
        let _ = std::fs::write(&stamp, &latest);
        if self.installed_version(own).await == latest {
            return Ok(Update::Current);
        }
        self.install("ytdlp").await?;
        Ok(Update::Updated)
    }

    /// Downloads a tool from its official release, checks it against the published checksum and installs it.
    pub async fn install(&self, name: &str) -> Result<(), String> {
        if let Some(f) = &self.installer {
            return f(name.to_string()).await;
        }
        let src = self
            .sources
            .get(name)
            .ok_or_else(|| "Outil inconnu".to_string())?
            .clone();
        if self.is_installing() {
            return Err("Une installation est déjà en cours".into());
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let tmp = self.dir.join(format!("{}.download", src.file));
        self.progress(name, 0, 0);
        let result = self.install_from(name, &src, &tmp).await;
        let _ = std::fs::remove_file(&tmp);
        *self.installing.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let _ = self.events.send(self.status());
        result
    }

    async fn install_from(&self, name: &str, src: &Source, tmp: &Path) -> Result<(), String> {
        let client = reqwest::Client::builder()
            .user_agent("tachy-download")
            .build()
            .map_err(|e| e.to_string())?;
        let sums = client
            .get(&src.sums)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;
        let expected = expected_hash(&sums, src)
            .ok_or_else(|| "Somme de contrôle introuvable : installation annulée".to_string())?;

        let mut res = client
            .get(&src.url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!(
                "Téléchargement impossible (HTTP {})",
                res.status().as_u16()
            ));
        }
        let total = res.content_length().unwrap_or(0);
        let mut hash = Sha256::new();
        let mut out = tokio::fs::File::create(tmp)
            .await
            .map_err(|e| e.to_string())?;
        let mut received = 0u64;
        let mut last = Instant::now();
        while let Some(chunk) = res.chunk().await.map_err(|e| e.to_string())? {
            hash.update(&chunk);
            out.write_all(&chunk).await.map_err(|e| e.to_string())?;
            received += chunk.len() as u64;
            if last.elapsed() >= Duration::from_millis(100) {
                self.progress(name, received, total);
                last = Instant::now();
            }
        }
        out.flush().await.map_err(|e| e.to_string())?;
        drop(out);
        self.progress(name, received, total);
        let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != expected {
            return Err("Somme de contrôle invalide : fichier rejeté".into());
        }

        let target = self.dir.join(&src.file);
        if src.zip {
            let out_dir = self.dir.join("unzip");
            let _ = std::fs::remove_dir_all(&out_dir);
            std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
            let unpacked = extract_zip(tmp, &out_dir).await;
            let result = unpacked.and_then(|()| {
                let found = find_file(&out_dir, &src.file.to_lowercase())
                    .ok_or_else(|| format!("{} absent de l’archive", src.file))?;
                std::fs::rename(found, &target).map_err(|e| e.to_string())
            });
            let _ = std::fs::remove_dir_all(&out_dir);
            result
        } else {
            std::fs::rename(tmp, &target).map_err(|e| e.to_string())
        }
    }
}

/// The hash the checksum list gives the download.
fn expected_hash(sums: &str, src: &Source) -> Option<String> {
    let line = match &src.sums_re {
        Some(_) => Some(sums),
        None => {
            let name = src.sum_name.as_deref().unwrap_or("");
            sums.lines().find(|l| l.trim().ends_with(name))
        }
    }?;
    let pattern = src.sums_re.as_deref().unwrap_or(r"^([0-9a-f]{64})");
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .ok()
        .or_else(|| Regex::new(r"^([0-9a-f]{64})").ok())?
        .captures(line)
        .map(|c| c[1].to_lowercase())
}

/// Unpacks a zip with the tar that Windows ships, by its full path: a GNU tar earlier on PATH (Git for Windows)
/// reads "C:\..." as a remote host.
async fn extract_zip(zip: &Path, to: &Path) -> Result<(), String> {
    let tar = crate::postprocess::system_tar();
    let mut cmd = tokio::process::Command::new(tar);
    cmd.arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(to)
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = cmd.output().await.map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_read_from_a_checksum_list_or_a_powershell_printout() {
        let h = "a".repeat(64);
        let g = "b".repeat(64);
        let list = format!("{g}  other.zip\n{h}  yt-dlp.exe\n");
        let yt = official_sources()["ytdlp"].clone();
        assert_eq!(expected_hash(&list, &yt), Some(h.clone()));
        assert_eq!(expected_hash(&format!("{g}  other.zip\n"), &yt), None);
        let deno = official_sources()["deno"].clone();
        let printout = format!(
            "Algorithm       Hash\n---------       ----\nSHA256          Hash : {}\n",
            h.to_uppercase()
        );
        assert_eq!(expected_hash(&printout, &deno), Some(h));
    }

    #[test]
    fn an_unknown_tool_has_no_path() {
        assert_eq!(Tools::new(std::env::temp_dir()).find("nothing"), None);
    }
}
