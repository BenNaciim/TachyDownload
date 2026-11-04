//! File-name and path helpers (`util.js`).

use std::path::{Path, PathBuf};

/// Windows device names that cannot be file names.
fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    let base = stem.as_str();
    matches!(base, "con" | "prn" | "aux" | "nul")
        || ((base.starts_with("com") || base.starts_with("lpt"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit()
            && base.as_bytes()[3] != b'0')
}

/// Turns any text into a name Windows accepts as a file name.
pub fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_slashes = false;
    for c in name.chars() {
        if c == '\\' || c == '/' {
            if !in_slashes {
                out.push('_');
            }
            in_slashes = true;
            continue;
        }
        in_slashes = false;
        out.push(
            if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || (c as u32) < 0x20 {
                '_'
            } else {
                c
            },
        );
    }
    let trimmed = out.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut n: String = trimmed.chars().take(180).collect();
    if n.is_empty() {
        n = "download".into();
    }
    if is_reserved(&n) {
        n.insert(0, '_');
    }
    n
}

/// `decodeURIComponent` that gives the text back unchanged when it is not valid.
pub fn safe_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            match hex {
                Some(b) => {
                    out.push(b);
                    i += 3;
                }
                None => return s.to_string(),
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

pub fn filename_from_content_disposition(header: &str) -> Option<String> {
    use regex::Regex;
    use std::sync::OnceLock;
    static STAR: OnceLock<Regex> = OnceLock::new();
    static QUOTED: OnceLock<Regex> = OnceLock::new();
    static PLAIN: OnceLock<Regex> = OnceLock::new();
    static ESC: OnceLock<Regex> = OnceLock::new();
    if header.is_empty() {
        return None;
    }
    let star =
        STAR.get_or_init(|| Regex::new(r"(?i)filename\*\s*=\s*([^']*)'[^']*'([^;]+)").unwrap());
    if let Some(c) = star.captures(header) {
        let v = c[2].trim();
        let v = v.strip_prefix('"').unwrap_or(v);
        let v = v.strip_suffix('"').unwrap_or(v);
        return Some(safe_decode(v));
    }
    let quoted =
        QUOTED.get_or_init(|| Regex::new(r#"(?i)filename\s*=\s*"((?:[^"\\]|\\.)*)""#).unwrap());
    if let Some(c) = quoted.captures(header) {
        let esc = ESC.get_or_init(|| Regex::new(r"\\(.)").unwrap());
        return Some(esc.replace_all(&c[1], "$1").into_owned());
    }
    let plain = PLAIN.get_or_init(|| Regex::new(r"(?i)filename\s*=\s*([^;]+)").unwrap());
    plain.captures(header).map(|c| c[1].trim().to_string())
}

pub fn filename_from_url(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let last = u.path().split('/').filter(|s| !s.is_empty()).next_back()?;
    Some(safe_decode(last))
}

/// `path.extname`: the last dot and what follows, "" for a leading dot only.
pub fn extname(name: &str) -> &str {
    match name.rfind('.') {
        Some(0) | None => "",
        Some(_) if name == ".." => "",
        Some(i) => &name[i..],
    }
}

/// Returns `dir/name`, or `dir/name (1).ext` etc. when the path is taken (a `.part` file counts as taken
/// unless `ignore_part`).
pub fn unique_path(dir: &Path, name: &str, ignore_part: bool) -> PathBuf {
    let ext = extname(name);
    let base = &name[..name.len() - ext.len()];
    let taken = |p: &Path| {
        p.exists() || (!ignore_part && PathBuf::from(format!("{}.part", p.display())).exists())
    };
    let mut candidate = dir.join(name);
    let mut i = 1;
    while taken(&candidate) {
        candidate = dir.join(format!("{base} ({i}){ext}"));
        i += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_matches_the_javascript_rules() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_filename("a:b*c?.zip"), "a_b_c_.zip");
        assert_eq!(sanitize_filename("  ..x..  "), "x");
        assert_eq!(sanitize_filename(""), "download");
        assert_eq!(sanitize_filename("..."), "download");
        assert_eq!(sanitize_filename("lpt1"), "_lpt1");
        assert_eq!(sanitize_filename("com0.txt"), "com0.txt");
        assert_eq!(sanitize_filename("a\\\\b//c"), "a_b_c");
        assert_eq!(sanitize_filename(&"x".repeat(300)).len(), 180);
    }

    #[test]
    fn content_disposition_forms() {
        assert_eq!(
            filename_from_content_disposition("attachment; filename=\"résumé.pdf\"").as_deref(),
            Some("résumé.pdf")
        );
        assert_eq!(
            filename_from_content_disposition("attachment; filename*=UTF-8''%E2%82%AC%20rates.txt")
                .as_deref(),
            Some("€ rates.txt")
        );
        assert_eq!(
            filename_from_content_disposition("attachment; filename=plain.bin; size=3").as_deref(),
            Some("plain.bin")
        );
        assert_eq!(
            filename_from_content_disposition("attachment; filename=\"a\\\"b.txt\"").as_deref(),
            Some("a\"b.txt")
        );
        assert_eq!(filename_from_content_disposition("inline"), None);
        assert_eq!(filename_from_content_disposition(""), None);
    }

    #[test]
    fn url_names_are_decoded() {
        assert_eq!(
            filename_from_url("http://h/files/sample.bin").as_deref(),
            Some("sample.bin")
        );
        assert_eq!(
            filename_from_url("http://h/a%20b.txt?x=1").as_deref(),
            Some("a b.txt")
        );
        assert_eq!(filename_from_url("http://h/"), None);
        assert_eq!(filename_from_url("not a url"), None);
    }

    #[test]
    fn unique_path_adds_a_counter() {
        let dir = std::env::temp_dir().join(format!("tachy-util-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a.bin", false), dir.join("a.bin"));
        std::fs::write(dir.join("a.bin"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.bin", false), dir.join("a (1).bin"));
        std::fs::write(dir.join("a (1).bin.part"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.bin", false), dir.join("a (2).bin"));
        assert_eq!(
            unique_path(&dir, "a.bin", true),
            dir.join("a (1).bin"),
            "a .part does not count when ignored"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extname_like_node() {
        assert_eq!(extname("a.b"), ".b");
        assert_eq!(extname(".bashrc"), "");
        assert_eq!(extname("noext"), "");
        assert_eq!(extname("a."), ".");
    }
}
