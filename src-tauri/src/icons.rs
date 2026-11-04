//! File icons for the list (`fileIconFor` in `main.js`): Windows picks the icon from the extension, the program that
//! opens the file. Programs and installers carry their own icon, so those use the real file once it exists. The
//! result is a PNG data URL, cached per extension (or per file for programs).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use tachy_core::state::Status;

#[derive(Default)]
pub struct IconCache(Mutex<HashMap<String, Option<String>>>);

const OWN_ICON_EXT: [&str; 4] = ["exe", "msi", "scr", "ico"];

/// What `file_icon` needs to know about a download.
pub struct Target<'a> {
    pub filename: &'a str,
    pub dir: &'a str,
    pub status: Status,
}

/// Lower-case extension of 1 to 10 letters or digits, "" otherwise.
fn plain_ext(filename: &str) -> String {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if (1..=10).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext
    } else {
        String::new()
    }
}

/// Where to ask Windows, and the key the answer is cached under: `(path, is_a_real_file, key)`.
fn lookup(t: &Target) -> (String, bool, String) {
    let ext = plain_ext(t.filename);
    let real = Path::new(t.dir).join(t.filename);
    if t.status == Status::Completed && OWN_ICON_EXT.contains(&ext.as_str()) && real.exists() {
        let p = real.to_string_lossy().into_owned();
        let key = format!("path:{p}");
        return (p, true, key);
    }
    // An empty stand-in with the right extension: only the extension matters for the lookup.
    let stand_in = format!("type.{}", if ext.is_empty() { "file" } else { &ext });
    (stand_in, false, format!("ext:{ext}"))
}

impl IconCache {
    pub async fn file_icon(&self, t: Target<'_>) -> Option<String> {
        let (path, real, key) = lookup(&t);
        if let Some(hit) = self.0.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return hit.clone();
        }
        let icon = tokio::task::spawn_blocking(move || shell::icon_data_url(&path, real))
            .await
            .ok()
            .flatten();
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, icon.clone());
        icon
    }
}

#[cfg(windows)]
mod shell {
    use base64::Engine;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
        GetDIBits, GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC,
    };
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES};
    use windows::Win32::UI::Shell::{
        SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};
    use windows::core::HSTRING;

    struct Rgba {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    }

    /// 32-bit top-down BGRA pixels of one bitmap, through GetDIBits.
    fn dib(bitmap: HBITMAP, width: i32, height: i32) -> Vec<u8> {
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (width * height * 4) as usize];
        unsafe {
            let dc = GetDC(None);
            GetDIBits(
                dc,
                bitmap,
                0,
                height as u32,
                Some(buf.as_mut_ptr().cast()),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            ReleaseDC(Some(HWND::default()), dc);
        }
        buf
    }

    /// The large icon of `path`. With `real` false the file need not exist: only its extension is used.
    fn shell_icon(path: &str, real: bool) -> Option<Rgba> {
        let mut info = SHFILEINFOW::default();
        let mut flags = SHGFI_ICON | SHGFI_LARGEICON;
        let attrs = if real {
            FILE_FLAGS_AND_ATTRIBUTES(0)
        } else {
            flags |= SHGFI_USEFILEATTRIBUTES;
            FILE_ATTRIBUTE_NORMAL
        };
        let ok = unsafe {
            SHGetFileInfoW(
                &HSTRING::from(path),
                attrs,
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                flags,
            )
        };
        if ok == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let mut ii = ICONINFO::default();
        let got = unsafe { GetIconInfo(info.hIcon, &mut ii) };
        let result = got.ok().map(|_| {
            let mut bm = BITMAP::default();
            unsafe {
                GetObjectW(
                    HGDIOBJ(ii.hbmColor.0),
                    std::mem::size_of::<BITMAP>() as i32,
                    Some((&mut bm as *mut BITMAP).cast()),
                )
            };
            let (w, h) = (bm.bmWidth, bm.bmHeight);
            let mut px = dib(ii.hbmColor, w, h);
            // Old-style icons have no alpha channel: take transparency from the mask instead.
            if px.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
                let mask = dib(ii.hbmMask, w, h);
                for (p, m) in px
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(mask.as_chunks::<4>().0)
                {
                    p[3] = if m[0] == 0 { 255 } else { 0 };
                }
            }
            for p in px.as_chunks_mut::<4>().0 {
                p.swap(0, 2); // BGRA -> RGBA
            }
            Rgba {
                width: w as u32,
                height: h as u32,
                pixels: px,
            }
        });
        unsafe {
            if !ii.hbmColor.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
            }
            if !ii.hbmMask.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
            }
            let _ = DestroyIcon(info.hIcon);
        }
        result
    }

    fn png(img: &Rgba) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .ok()?
            .write_image_data(&img.pixels)
            .ok()?;
        Some(out)
    }

    pub fn icon_data_url(path: &str, real: bool) -> Option<String> {
        let bytes = png(&shell_icon(path, real)?)?;
        Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }
}

#[cfg(not(windows))]
mod shell {
    pub fn icon_data_url(_path: &str, _real: bool) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t<'a>(filename: &'a str, dir: &'a str, status: Status) -> Target<'a> {
        Target {
            filename,
            dir,
            status,
        }
    }

    #[test]
    fn the_icon_follows_the_extension_and_ignores_odd_ones() {
        assert_eq!(lookup(&t("a.PDF", "", Status::Queued)).2, "ext:pdf");
        assert_eq!(lookup(&t("a.tar.gz", "", Status::Queued)).2, "ext:gz");
        assert_eq!(lookup(&t("noext", "", Status::Queued)).2, "ext:");
        assert_eq!(lookup(&t("weird.é☃", "", Status::Queued)).2, "ext:");
        assert_eq!(lookup(&t("long.abcdefghijk", "", Status::Queued)).2, "ext:");
        assert_eq!(lookup(&t("noext", "", Status::Queued)).0, "type.file");
    }

    #[test]
    fn a_finished_program_uses_its_own_icon_others_share_the_type_icon() {
        let dir = std::env::temp_dir().join(format!("tachy-icons-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("setup.exe"), "x").unwrap();
        let d = dir.to_string_lossy().into_owned();
        let own = lookup(&t("setup.exe", &d, Status::Completed));
        assert!(own.1 && own.2.starts_with("path:"));
        assert_eq!(
            lookup(&t("setup.exe", &d, Status::Downloading)).2,
            "ext:exe"
        );
        assert_eq!(
            lookup(&t("missing.exe", &d, Status::Completed)).2,
            "ext:exe"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_gives_a_png_for_a_type_and_the_answer_is_cached() {
        let cache = IconCache::default();
        let url = cache.file_icon(t("report.pdf", "", Status::Queued)).await;
        let url = url.expect("an icon for .pdf");
        assert!(url.starts_with("data:image/png;base64,"));
        assert_eq!(cache.0.lock().unwrap().len(), 1);
        assert_eq!(
            cache.file_icon(t("other.pdf", "", Status::Queued)).await,
            Some(url)
        );
        assert_eq!(cache.0.lock().unwrap().len(), 1, "one entry per extension");
    }
}
