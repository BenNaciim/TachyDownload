//! The data folder (`%APPDATA%\Tachy Download`, the same as the Electron version), the extension token and the
//! one-time move from the folder of the old "Download Manager" name.

use std::path::{Path, PathBuf};

pub const TOKEN_FILE: &str = "extension-token.txt";

/// A token of at least 32 characters is kept; otherwise a new one is made (24 random bytes as hex, like before) and
/// written for the user to copy into the extension.
pub fn load_token(dir: &Path) -> std::io::Result<String> {
    if let Ok(text) = std::fs::read_to_string(dir.join(TOKEN_FILE)) {
        let t = text.trim();
        if t.len() >= 32 {
            return Ok(t.to_string());
        }
    }
    // Two v4 UUIDs hold 32 random bytes; 24 of them make the token.
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    let token: String = bytes[..24].iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(TOKEN_FILE), &token)?;
    Ok(token)
}

/// The old app was called "Download Manager": bring its settings, list, token and tools along, once. Nothing is
/// overwritten. Returns whether something was copied.
pub fn migrate_old_data(old: &Path, dest: &Path) -> bool {
    if dest.join(TOKEN_FILE).exists() || !old.join(TOKEN_FILE).exists() {
        return false;
    }
    copy_missing(old, dest).is_ok()
}

fn copy_missing(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_missing(&entry.path(), &target)?;
        } else if !target.exists() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// `--data-dir <folder>` replaces the default folder (the end-to-end checks use a throwaway one).
pub fn data_dir_override(args: &[String]) -> Option<PathBuf> {
    let i = args.iter().position(|a| a == "--data-dir")?;
    args.get(i + 1).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tachy-data-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_token_is_made_once_and_kept() {
        let d = tmp("token");
        let first = load_token(&d).unwrap();
        assert_eq!(first.len(), 48);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(
            load_token(&d).unwrap(),
            first,
            "the next start reads it back"
        );
        std::fs::write(d.join(TOKEN_FILE), "short").unwrap();
        let fresh = load_token(&d).unwrap();
        assert_eq!(
            fresh.len(),
            48,
            "a token shorter than 32 characters is replaced"
        );
        assert_ne!(fresh, first);
    }

    #[test]
    fn two_tokens_are_never_alike() {
        assert_ne!(
            load_token(&tmp("a")).unwrap(),
            load_token(&tmp("b")).unwrap()
        );
    }

    #[test]
    fn the_old_folder_is_copied_once_and_never_over_newer_files() {
        let (old, dest) = (tmp("old"), tmp("new").join("data"));
        std::fs::write(old.join(TOKEN_FILE), "t".repeat(40)).unwrap();
        std::fs::write(old.join("downloads.json"), "[1]").unwrap();
        std::fs::create_dir_all(old.join("tools")).unwrap();
        std::fs::write(old.join("tools").join("yt-dlp.exe"), "x").unwrap();
        assert!(migrate_old_data(&old, &dest));
        assert_eq!(
            std::fs::read_to_string(dest.join("downloads.json")).unwrap(),
            "[1]"
        );
        assert!(dest.join("tools").join("yt-dlp.exe").exists());
        // the new folder now has a token: a second start leaves it alone
        std::fs::write(old.join("downloads.json"), "[2]").unwrap();
        assert!(!migrate_old_data(&old, &dest));
        assert_eq!(
            std::fs::read_to_string(dest.join("downloads.json")).unwrap(),
            "[1]"
        );
        // nothing to bring when the old folder has no token
        assert!(!migrate_old_data(&tmp("empty"), &tmp("elsewhere")));
    }

    #[test]
    fn data_dir_can_be_overridden_on_the_command_line() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            data_dir_override(&a(&["app.exe", "--data-dir", "D:\\x"])),
            Some(PathBuf::from("D:\\x"))
        );
        assert_eq!(data_dir_override(&a(&["app.exe", "--hidden"])), None);
        assert_eq!(data_dir_override(&a(&["app.exe", "--data-dir"])), None);
    }
}
