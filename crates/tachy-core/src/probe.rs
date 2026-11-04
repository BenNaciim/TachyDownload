//! Asking a server about a file (`probe.js`).

use crate::errors::{Fail, Result, TaskError};
use crate::net::{Headers, Net};
use crate::util::{filename_from_content_disposition, filename_from_url, sanitize_filename};
use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct ProbeInfo {
    pub final_url: String,
    pub size: Option<u64>,
    pub range_ok: bool,
    pub filename: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

fn with_range(headers: &Headers) -> Headers {
    let mut h: Headers = headers
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("range"))
        .cloned()
        .collect();
    h.push(("Range".into(), "bytes=0-0".into()));
    h
}

fn header(res: &reqwest::Response, name: &str) -> Option<String> {
    res.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Asks the server for the first byte only. A 206 answer proves range support and gives the total size in
/// `Content-Range`; a plain 200 means a single stream.
pub async fn probe(
    net: &Net,
    url: &str,
    headers: &Headers,
    cancel: &CancellationToken,
) -> Result<ProbeInfo> {
    static RANGE: OnceLock<Regex> = OnceLock::new();
    let res = net.get(url, &with_range(headers), cancel).await?;
    let status = res.status().as_u16();
    if status >= 400 {
        return Err(Fail::Error(TaskError::http(status)));
    }
    let mut size = None;
    let mut range_ok = false;
    if status == 206 {
        let re = RANGE.get_or_init(|| Regex::new(r"(?i)bytes\s+\d+-\d+/(\d+)").unwrap());
        if let Some(c) = header(&res, "content-range")
            .and_then(|v| re.captures(&v).and_then(|c| c[1].parse::<u64>().ok()))
        {
            size = Some(c);
            range_ok = true;
        }
    }
    if !range_ok {
        size = header(&res, "content-length")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|n| *n > 0);
    }
    let final_url = res.url().to_string();
    let name = header(&res, "content-disposition")
        .and_then(|v| filename_from_content_disposition(&v))
        .filter(|n| !n.is_empty())
        .or_else(|| filename_from_url(&final_url))
        .unwrap_or_else(|| "download".into());
    Ok(ProbeInfo {
        final_url,
        size,
        range_ok,
        filename: sanitize_filename(&name),
        etag: header(&res, "etag"),
        last_modified: header(&res, "last-modified"),
    })
}

/// True when the address answers with a web page rather than a file: the link of a video page on a site the
/// app does not know by name, which only yt-dlp can turn into a video. Any failure counts as "no".
pub async fn is_web_page(net: &Net, url: &str, headers: &Headers) -> bool {
    let cancel = CancellationToken::new();
    let work = async {
        let Ok(res) = net.get(url, &with_range(headers), &cancel).await else {
            return false;
        };
        let html = header(&res, "content-type").is_some_and(|t| {
            let t = t.to_ascii_lowercase();
            t.starts_with("text/html") || t.starts_with("application/xhtml")
        });
        res.status().as_u16() < 400 && html
    };
    tokio::time::timeout(Duration::from_secs(8), work)
        .await
        .unwrap_or(false)
}
