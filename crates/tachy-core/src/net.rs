//! The HTTP client every download uses (`net.js`): HTTP/1.1 only, so that each range gets its own TCP
//! connection (HTTP/2 would multiplex them all on one pipe and defeat the split), no automatic decompression
//! and no cookie jar (`Cookie` is sent by hand).

use crate::errors::{Fail, MSG_BAD_PORT, MSG_INVALID_URL, Result, TaskError};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Request headers as name/value pairs, in the order they were set.
pub type Headers = Vec<(String, String)>;

/// A proxy address as typed by the user: "host:port" is read as http://host:port. Empty means no proxy.
pub fn normalize_proxy(value: &str) -> std::result::Result<String, String> {
    let raw = value.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }
    let has_scheme = {
        let mut chars = raw.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && raw.find("://").is_some_and(|i| {
                raw[..i]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
            })
    };
    let candidate = if has_scheme {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let u = url::Url::parse(&candidate).map_err(|_| "Adresse du proxy invalide".to_string())?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return Err("Le proxy doit être de type http ou https".into());
    }
    if u.host_str().is_none_or(str::is_empty) {
        return Err("Adresse du proxy invalide".into());
    }
    Ok(u.to_string().trim_end_matches('/').to_string())
}

/// The ports `fetch` refuses (the "bad ports" of the Fetch standard).
fn is_bad_port(port: u16) -> bool {
    matches!(
        port,
        1 | 7
            | 9
            | 11
            | 13
            | 15
            | 17
            | 19
            | 20
            | 21
            | 22
            | 23
            | 25
            | 37
            | 42
            | 43
            | 53
            | 69
            | 77
            | 79
            | 87
            | 95
            | 101
            | 102
            | 103
            | 104
            | 109
            | 110
            | 111
            | 113
            | 115
            | 117
            | 119
            | 123
            | 135
            | 137
            | 139
            | 143
            | 161
            | 179
            | 389
            | 427
            | 465
            | 512
            | 513
            | 514
            | 515
            | 526
            | 530
            | 531
            | 532
            | 540
            | 548
            | 554
            | 556
            | 563
            | 587
            | 601
            | 636
            | 989
            | 990
            | 993
            | 995
            | 1719
            | 1720
            | 1723
            | 2049
            | 3659
            | 4045
            | 4190
            | 5060
            | 5061
            | 6000
            | 6566
            | 6665..=6669 | 6679 | 6697 | 10080
    )
}

#[derive(Clone)]
pub struct Net {
    client: reqwest::Client,
    proxy: String,
}

impl Net {
    /// `proxy` is a normalised address or "".
    pub fn new(proxy: &str) -> Self {
        let mut b = reqwest::Client::builder()
            .http1_only()
            .connect_timeout(Duration::from_secs(15))
            // No data for 60 s counts as a lost connection (Node waits five minutes).
            .read_timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::limited(20))
            .user_agent("Tachy Download")
            .no_proxy();
        if !proxy.is_empty() {
            if let Ok(p) = reqwest::Proxy::all(proxy) {
                b = b.proxy(p);
            }
        }
        let client = b.build().expect("the HTTP client could not be built");
        Self {
            client,
            proxy: proxy.to_string(),
        }
    }

    pub fn proxy(&self) -> &str {
        &self.proxy
    }

    /// A GET with these headers; redirects are followed and the final address is `response.url()`.
    pub async fn get(
        &self,
        url: &str,
        headers: &Headers,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response> {
        let parsed =
            url::Url::parse(url).map_err(|_| Fail::Error(TaskError::fatal(MSG_INVALID_URL)))?;
        if parsed.port().is_some_and(is_bad_port) {
            return Err(Fail::Error(TaskError::fatal(MSG_BAD_PORT)));
        }
        let mut map = HeaderMap::new();
        map.insert(
            reqwest::header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        for (k, v) in headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_bytes(v.as_bytes()),
            ) {
                map.insert(name, value);
            }
        }
        let send = self.client.get(parsed).headers(map).send();
        tokio::select! {
            r = send => r.map_err(|e| Fail::Error(TaskError::from_reqwest(&e))),
            _ = cancel.cancelled() => Err(Fail::Cancelled),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proxy_address_is_cleaned_up_and_an_unusable_one_is_refused() {
        assert_eq!(normalize_proxy("").unwrap(), "");
        assert_eq!(normalize_proxy("  ").unwrap(), "");
        assert_eq!(
            normalize_proxy("proxy.local:3128").unwrap(),
            "http://proxy.local:3128"
        );
        assert_eq!(
            normalize_proxy("http://user:pw@10.0.0.1:8080/").unwrap(),
            "http://user:pw@10.0.0.1:8080"
        );
        assert!(
            normalize_proxy("socks5://127.0.0.1:1080")
                .unwrap_err()
                .contains("http ou https")
        );
        assert!(normalize_proxy("http://").unwrap_err().contains("invalide"));
        assert!(
            normalize_proxy("ftp://x")
                .unwrap_err()
                .contains("http ou https")
        );
    }

    #[test]
    fn bad_ports_are_the_fetch_ones() {
        for p in [22u16, 25, 6667, 10080, 6000] {
            assert!(is_bad_port(p), "{p}");
        }
        for p in [80u16, 443, 6800, 8080, 49152] {
            assert!(!is_bad_port(p), "{p}");
        }
    }

    #[tokio::test]
    async fn a_blocked_port_is_refused_before_any_request() {
        let err = Net::new("")
            .get("http://127.0.0.1:22/x", &vec![], &CancellationToken::new())
            .await
            .unwrap_err();
        match err {
            Fail::Error(e) => {
                assert_eq!(e.message, MSG_BAD_PORT);
                assert!(e.fatal);
            }
            Fail::Cancelled => panic!("cancelled"),
        }
    }
}
