//! What happens once a download is finished (`postprocess.js`): a warning about a copy of a file already received,
//! and unpacking archives.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// SHA-256 of a file, read in pieces so a large file never sits in memory.
pub fn file_hash(file: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(file)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Looks among the other finished downloads for a file with the same content. Sizes are compared first, so only
/// files of the same size are ever read. Returns the path of the earlier copy.
pub fn find_duplicate(file: &Path, others: &[PathBuf]) -> Option<PathBuf> {
    let size = std::fs::metadata(file).ok()?.len();
    if size == 0 {
        return None;
    }
    let me = std::path::absolute(file).ok()?;
    let same: Vec<&PathBuf> = others
        .iter()
        .filter(|p| {
            std::path::absolute(p).is_ok_and(|a| a != me)
                && std::fs::metadata(p).is_ok_and(|m| m.len() == size)
        })
        .collect();
    if same.is_empty() {
        return None;
    }
    let mine = file_hash(file).ok()?;
    same.into_iter()
        .find(|p| file_hash(p).ok().as_deref() == Some(mine.as_str()))
        .cloned()
}

/// The tar that ships with Windows 10 and later (bsdtar, which reads zip and 7z), by its full path: a tar earlier
/// in the PATH, such as the one from Git, is a different program that reads "C:" as a host name.
pub fn system_tar() -> PathBuf {
    #[cfg(windows)]
    {
        let own =
            PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()))
                .join("System32")
                .join("tar.exe");
        if own.exists() {
            return own;
        }
    }
    PathBuf::from("tar")
}

const ARCHIVE_ENDINGS: [&str; 7] = [
    ".zip", ".7z", ".tar", ".tgz", ".tar.gz", ".tar.xz", ".tar.bz2",
];

fn archive_ending(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    // the longest ending first: "a.tar.gz" is a ".tar.gz", not a ".gz"
    ARCHIVE_ENDINGS
        .iter()
        .filter(|e| lower.ends_with(**e))
        .max_by_key(|e| e.len())
        .copied()
}

pub fn is_archive(name: &str) -> bool {
    archive_ending(name).is_some()
}

/// An archive's folder name: the file name without its archive extension.
fn archive_stem(name: &str) -> &str {
    match archive_ending(name) {
        Some(e) => &name[..name.len() - e.len()],
        None => name,
    }
}

/// Unpacks an archive into a new folder named after it, next to the archive, with the system tar. When the folder
/// exists the next free "name (1)" is used, so two archives never merge. A failed unpacking removes the folder it
/// made. Returns the folder.
pub async fn extract_archive(file: &Path, tar: Option<&Path>) -> Result<PathBuf, String> {
    let dir = file.parent().unwrap_or(Path::new("."));
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = match archive_stem(&name) {
        "" => "archive",
        s => s,
    };
    let mut dest = dir.join(stem);
    let mut i = 1;
    while dest.exists() {
        dest = dir.join(format!("{stem} ({i})"));
        i += 1;
    }
    std::fs::create_dir(&dest).map_err(|e| e.to_string())?;
    let tar = tar.map(Path::to_path_buf).unwrap_or_else(system_tar);
    let mut cmd = tokio::process::Command::new(tar);
    cmd.arg("-xf")
        .arg(file)
        .arg("-C")
        .arg(&dest)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let run = tokio::time::timeout(std::time::Duration::from_secs(10 * 60), cmd.output()).await;
    let failure = match run {
        Ok(Ok(out)) if out.status.success() => return Ok(dest),
        Ok(Ok(out)) => {
            let text = String::from_utf8_lossy(&out.stderr);
            text.lines()
                .next()
                .unwrap_or("")
                .trim()
                .chars()
                .take(200)
                .collect::<String>()
        }
        Ok(Err(e)) => e.to_string(),
        Err(_) => "Extraction trop longue".to_string(),
    };
    let _ = std::fs::remove_dir_all(&dest);
    Err(if failure.is_empty() {
        "Extraction impossible".to_string()
    } else {
        failure
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_is_named_after_the_archive() {
        assert_eq!(archive_stem("pack.zip"), "pack");
        assert_eq!(archive_stem("data.tar.gz"), "data");
        assert_eq!(archive_stem("DATA.TAR.XZ"), "DATA");
        assert_eq!(archive_stem("plain.txt"), "plain.txt");
    }

    #[test]
    fn a_hash_is_the_sha256_of_the_content() {
        let f = std::env::temp_dir().join(format!("tachy-hash-{}", std::process::id()));
        std::fs::write(&f, "abc").unwrap();
        assert_eq!(
            file_hash(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_file(f).unwrap();
    }
}
