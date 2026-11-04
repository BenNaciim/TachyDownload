//! THROWAWAY (proof S5, first half). The shell icon of a file type (or of a real file) as a PNG, like Electron's
//! `app.getFileIcon(path, { size: 'large' }).toDataURL()` that `src/main.js` uses.
//!
//!   spike-icons <out dir> pdf zip mp4 txt xyz          icons by extension (an empty stand-in name, no file needed)
//!   spike-icons <out dir> C:\Windows\notepad.exe       the icon of a real file (executables carry their own)

use std::path::Path;

use base64::Engine;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, HGDIOBJ, ReleaseDC,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES};
use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};
use windows::core::HSTRING;

struct Rgba {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// 32-bit top-down BGRA pixels of one bitmap, through GetDIBits.
fn dib(bitmap: windows::Win32::Graphics::Gdi::HBITMAP, width: i32, height: i32) -> Vec<u8> {
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height, // top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let mut buf = vec![0u8; (width * height * 4) as usize];
    unsafe {
        let dc = GetDC(None);
        GetDIBits(dc, bitmap, 0, height as u32, Some(buf.as_mut_ptr().cast()), &mut bmi, DIB_RGB_COLORS);
        ReleaseDC(Some(HWND::default()), dc);
    }
    buf
}

/// The icon of `path`. With `stand_in` the file need not exist: only its extension is used.
fn shell_icon(path: &str, stand_in: bool) -> Option<Rgba> {
    let mut info = SHFILEINFOW::default();
    let mut flags = SHGFI_ICON | SHGFI_LARGEICON;
    let attrs = if stand_in {
        flags |= SHGFI_USEFILEATTRIBUTES;
        FILE_ATTRIBUTE_NORMAL
    } else {
        FILE_FLAGS_AND_ATTRIBUTES(0)
    };
    let ok = unsafe { SHGetFileInfoW(&HSTRING::from(path), attrs, Some(&mut info), std::mem::size_of::<SHFILEINFOW>() as u32, flags) };
    if ok == 0 || info.hIcon.is_invalid() {
        return None;
    }
    let mut ii = ICONINFO::default();
    let got = unsafe { GetIconInfo(info.hIcon, &mut ii) };
    let result = got.ok().map(|_| {
        let mut bm = BITMAP::default();
        unsafe { GetObjectW(HGDIOBJ(ii.hbmColor.0), std::mem::size_of::<BITMAP>() as i32, Some((&mut bm as *mut BITMAP).cast())) };
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        let mut px = dib(ii.hbmColor, w, h);
        // Old-style icons have no alpha channel: take transparency from the mask instead.
        if px.chunks_exact(4).all(|p| p[3] == 0) {
            let mask = dib(ii.hbmMask, w, h);
            for (p, m) in px.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                p[3] = if m[0] == 0 { 255 } else { 0 };
            }
        }
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2); // BGRA -> RGBA
        }
        Rgba { width: w as u32, height: h as u32, pixels: px }
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

fn png(img: &Rgba) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&img.pixels).unwrap();
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().expect("usage: spike-icons <out dir> <ext|path>...");
    std::fs::create_dir_all(&out_dir).unwrap();
    for what in args {
        let real = Path::new(&what).exists();
        let target = if real { what.clone() } else { format!("type.{what}") };
        match shell_icon(&target, !real) {
            Some(img) => {
                let bytes = png(&img);
                let name = what.replace(['\\', '/', ':'], "_");
                std::fs::write(Path::new(&out_dir).join(format!("{name}.png")), &bytes).unwrap();
                let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
                println!("{what}: {}x{} px, {} bytes PNG, data URL {} chars", img.width, img.height, bytes.len(), url.len());
            }
            None => println!("{what}: no icon"),
        }
    }
}
