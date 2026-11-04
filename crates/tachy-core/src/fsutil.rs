//! Disk helpers: positional writes into the `.part` file, and the end of a download.

use crate::ctx::Ctx;
use crate::state::TaskState;
use crate::util::unique_path;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// Writes all of `buf` at `offset`, without moving any file cursor (several connections share one handle).
pub fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    while !buf.is_empty() {
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_write(file, buf, offset)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::write_at(file, buf, offset)?;
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::WriteZero));
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// End of a download, shared by every engine: moves the finished file `from` into its final folder (the type
/// folder when that setting is on), under a free name, then records where it ended up on the task state. A file
/// already in the right place is left alone. Returns the final path.
/// A download redone on purpose (`replaces`) takes the place of the file it replaces; when that file is in use
/// and cannot be replaced, both are kept.
pub fn settle_finished(
    ctx: &Ctx,
    s: &mut TaskState,
    from: &Path,
    name: &str,
) -> io::Result<PathBuf> {
    let final_dir = ctx.final_dir(s, name);
    std::fs::create_dir_all(&final_dir)?;
    let wanted = final_dir.join(name);
    let mut target = from.to_path_buf();
    if !same_path(&wanted, from) {
        let mut placed = None;
        if let Some(replaces) = &s.replaces
            && same_path(Path::new(replaces), &wanted)
            && std::fs::rename(from, &wanted).is_ok()
        {
            placed = Some(wanted.clone()); // replaces the old file in one step
        }
        target = match placed {
            Some(p) => p,
            None => {
                let free = unique_path(&final_dir, name, true);
                std::fs::rename(from, &free)?;
                free
            }
        };
    }
    if let Some(replaces) = s.replaces.take() {
        // The new file may have another extension than the old one (a stream converted to mp4): the old one goes too.
        if !same_path(Path::new(&replaces), &target) {
            let _ = std::fs::remove_file(replaces);
        }
    }
    s.dir = final_dir.to_string_lossy().into_owned();
    s.filename = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    s.completed_at = Some(now_ms());
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_land_at_their_offsets_in_any_order() {
        let dir = std::env::temp_dir().join(format!("tachy-fs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.part");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&p)
            .unwrap();
        f.set_len(10).unwrap();
        write_all_at(&f, b"world", 5).unwrap();
        write_all_at(&f, b"hello", 0).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"helloworld");
        std::fs::remove_dir_all(dir).ok();
    }
}
