//! The one list of file types (the same as `ui/shared/filetypes.js`, which the window keeps using).
//! The engine files a download into the type's `folder`; the window groups by `kind` and shows `label`.

pub struct FileType {
    pub kind: &'static str,
    pub label: &'static str,
    pub folder: &'static str,
    pub exts: &'static [&'static str],
}

pub const FILE_TYPES: &[FileType] = &[
    FileType {
        kind: "video",
        label: "VID",
        folder: "Vidéos",
        exts: &[
            "mp4", "mkv", "avi", "mov", "webm", "wmv", "flv", "m4v", "ts",
        ],
    },
    FileType {
        kind: "audio",
        label: "MP3",
        folder: "Musique",
        exts: &["mp3", "wav", "flac", "aac", "ogg", "m4a", "opus"],
    },
    FileType {
        kind: "image",
        label: "IMG",
        folder: "Images",
        exts: &[
            "jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "avif", "tif", "tiff",
        ],
    },
    FileType {
        kind: "doc",
        label: "DOC",
        folder: "Documents",
        exts: &[
            "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "odt", "csv", "epub",
        ],
    },
    FileType {
        kind: "archive",
        label: "ZIP",
        folder: "Archives",
        exts: &["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz"],
    },
    FileType {
        kind: "app",
        label: "APP",
        folder: "Programmes",
        exts: &[
            "exe", "msi", "msix", "iso", "dmg", "apk", "deb", "rpm", "img",
        ],
    },
];

pub const OTHER_FOLDER: &str = "Autres";

/// Lower-case extension, ignoring a trailing ".part"; "" when there is none.
pub fn ext_of(filename: &str) -> String {
    let name = filename.strip_suffix(".part").unwrap_or(filename);
    match name.rfind('.') {
        Some(i)
            if i + 1 < name.len() && name[i + 1..].chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            name[i + 1..].to_ascii_lowercase()
        }
        _ => String::new(),
    }
}

/// The entry of `FILE_TYPES` for a file name, or `None` for an unknown type.
pub fn type_of(filename: &str) -> Option<&'static FileType> {
    let ext = ext_of(filename);
    FILE_TYPES.iter().find(|t| t.exts.contains(&ext.as_str()))
}

/// Folder name for a file type: every type gets one, unknown ones go to "Autres".
pub fn category_for(filename: &str) -> &'static str {
    type_of(filename).map(|t| t.folder).unwrap_or(OTHER_FOLDER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_ignores_part_and_case() {
        assert_eq!(ext_of("a.MP4"), "mp4");
        assert_eq!(ext_of("a.zip.part"), "zip");
        assert_eq!(ext_of("noext"), "");
        assert_eq!(ext_of("trailing."), "");
        assert_eq!(ext_of("x.tar.gz"), "gz");
    }

    #[test]
    fn folders_follow_the_type_and_unknown_goes_to_autres() {
        assert_eq!(category_for("clip.mkv"), "Vidéos");
        assert_eq!(category_for("song.flac"), "Musique");
        assert_eq!(category_for("setup.EXE"), "Programmes");
        assert_eq!(category_for("data.xyz"), OTHER_FOLDER);
        assert_eq!(category_for("noext"), OTHER_FOLDER);
    }

    /// The Rust table must stay identical to the one the window loads.
    #[test]
    fn table_matches_the_javascript_one() {
        let js = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../ui/shared/filetypes.js"
        ))
        .unwrap();
        for t in FILE_TYPES {
            let exts: Vec<String> = t.exts.iter().map(|e| format!("'{e}'")).collect();
            let line = format!(
                "{{ kind: '{}', label: '{}', folder: '{}', exts: [{}] }}",
                t.kind,
                t.label,
                t.folder,
                exts.join(", ")
            );
            assert!(
                js.contains(&line),
                "missing or different in filetypes.js: {line}"
            );
        }
        assert!(js.contains(&format!("const OTHER_FOLDER = '{OTHER_FOLDER}'")));
    }
}
