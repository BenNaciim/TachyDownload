//! Filing rules, one per line: `pattern = folder`. A pattern starting with a dot or `*.` matches a file
//! extension; anything else matches a site (the host itself or any of its subdomains). The folder is absolute
//! or relative to the download folder. Lines starting with # are comments.

use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    Ext { ext: String, folder: String },
    Host { host: String, folder: String },
}

pub fn parse_rules(text: &str) -> Vec<Rule> {
    static LINE: OnceLock<Regex> = OnceLock::new();
    static EXT: OnceLock<Regex> = OnceLock::new();
    let line_re = LINE.get_or_init(|| Regex::new(r"^\s*([^#=][^=]*?)\s*=\s*(.+?)\s*$").unwrap());
    let ext_re = EXT.get_or_init(|| Regex::new(r"^\*?\.([a-z0-9]{1,10})$").unwrap());
    let mut rules = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some(m) = line_re.captures(line) else {
            continue;
        };
        let pattern = m[1].to_lowercase();
        let folder = m[2].trim_matches('"').to_string();
        if let Some(e) = ext_re.captures(&pattern) {
            rules.push(Rule::Ext {
                ext: e[1].to_string(),
                folder,
            });
        } else {
            let host = pattern
                .strip_prefix("*.")
                .or_else(|| pattern.strip_prefix('.'))
                .unwrap_or(&pattern)
                .to_string();
            rules.push(Rule::Host { host, folder });
        }
    }
    rules
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .unwrap_or_default()
}

fn on_site(host: &str, site: &str) -> bool {
    !host.is_empty() && (host == site || host.ends_with(&format!(".{site}")))
}

/// The folder the first matching rule sends this file to, or `None`.
pub fn folder_for(
    rules: &[Rule],
    filename: &str,
    url: &str,
    page_url: &str,
    base_dir: &Path,
) -> Option<PathBuf> {
    let ext = crate::util::extname(filename)
        .trim_start_matches('.')
        .to_lowercase();
    let hosts = [host_of(url), host_of(page_url)];
    for r in rules {
        let (hit, folder) = match r {
            Rule::Ext { ext: e, folder } => (*e == ext, folder),
            Rule::Host { host, folder } => (hosts.iter().any(|h| on_site(h, host)), folder),
        };
        if hit {
            let f = PathBuf::from(folder);
            return Some(if f.is_absolute() { f } else { base_dir.join(f) });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extension_and_site_rules() {
        let r = parse_rules(
            "# comment\n.PDF = Docs\n*.zip=\"D:\\Zips\"\n\nexample.com = Site\n*.cdn.net = Cdn\nbad line\n",
        );
        assert_eq!(
            r,
            vec![
                Rule::Ext {
                    ext: "pdf".into(),
                    folder: "Docs".into()
                },
                Rule::Ext {
                    ext: "zip".into(),
                    folder: "D:\\Zips".into()
                },
                Rule::Host {
                    host: "example.com".into(),
                    folder: "Site".into()
                },
                Rule::Host {
                    host: "cdn.net".into(),
                    folder: "Cdn".into()
                },
            ]
        );
    }

    #[test]
    fn first_match_wins_and_relative_folders_hang_off_the_base() {
        let rules = parse_rules(".pdf = Docs\nexample.com = Site\n");
        let base = Path::new("C:\\dl");
        assert_eq!(
            folder_for(&rules, "a.PDF", "http://x/a.pdf", "", base),
            Some(base.join("Docs"))
        );
        assert_eq!(
            folder_for(&rules, "a.bin", "http://cdn.example.com/a.bin", "", base),
            Some(base.join("Site"))
        );
        assert_eq!(
            folder_for(
                &rules,
                "a.bin",
                "http://x/a.bin",
                "https://example.com/page",
                base
            ),
            Some(base.join("Site"))
        );
        assert_eq!(
            folder_for(&rules, "a.bin", "http://notexample.com/a.bin", "", base),
            None
        );
    }
}
