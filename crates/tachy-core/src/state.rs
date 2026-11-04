//! The state of one download: plain data, exactly what `downloads.json` stores (annex C of the design) and the
//! base of what the window receives. Unknown fields are kept (`extra`), so a file written by another version
//! survives a load and a save.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A byte range of the file, with how far it has been written. `end` is inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub pos: u64,
}

/// The same value JavaScript's `Number.MAX_SAFE_INTEGER` has: the end of a stream of unknown size.
pub const UNKNOWN_END: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Queued,
    Scheduled,
    Downloading,
    Paused,
    Completed,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    File,
    Hls,
    Ytdlp,
}

/// Headers replayed with every request of the download.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReplayHeaders {
    pub referer: String,
    pub origin: String,
    pub cookie: String,
    pub user_agent: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskState {
    pub id: String,
    pub kind: Kind,
    pub own_folder: bool,
    pub page_url: String,
    pub auto_refresh: bool,
    pub needs_refresh: bool,
    pub url: String,
    pub final_url: Option<String>,
    pub dir: String,
    pub filename: String,
    pub filename_locked: bool,
    pub size: Option<u64>,
    pub range_ok: bool,
    pub segments: Option<Vec<Segment>>,
    pub connections: u32,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub headers: Option<ReplayHeaders>,
    pub status: Status,
    pub error: Option<String>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
    pub start_at: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none", rename = "speedLimitKBps")]
    pub speed_limit_kbps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracting: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_done: Option<bool>,
    // Kinds other than plain files keep their own bookkeeping here until their modules type it (M2, M3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hls: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ytd: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frag: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yt_opts: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_stem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct: Option<bool>,

    /// Fields this version does not know: kept as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl TaskState {
    /// Bytes written so far, over every segment.
    pub fn downloaded(&self) -> u64 {
        self.segments
            .as_ref()
            .map_or(0, |s| s.iter().map(|g| g.pos.saturating_sub(g.start)).sum())
    }
}

/// What the window receives: the state plus the computed fields (design 5.2).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(flatten)]
    pub state: TaskState,
    pub downloaded: u64,
    /// Bytes per second over the last three seconds, 0 when not downloading.
    pub speed: f64,
    pub eta: Option<u64>,
    /// Fields only some kinds have (`hlsProgress`, `hlsTotal`...).
    #[serde(flatten)]
    pub more: Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_fields_survive_a_round_trip() {
        let raw = json!({ "id": "a", "kind": "file", "status": "paused", "url": "http://h/x", "mystery": { "n": 1 }, "futureFlag": true });
        let s: TaskState = serde_json::from_value(raw).unwrap();
        assert_eq!(s.status, Status::Paused);
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["mystery"], json!({ "n": 1 }));
        assert_eq!(back["futureFlag"], json!(true));
    }

    #[test]
    fn missing_fields_take_defaults_and_names_are_camel_case() {
        let s: TaskState = serde_json::from_value(json!({ "id": "a" })).unwrap();
        assert_eq!(s.status, Status::Queued);
        assert_eq!(s.kind, Kind::File);
        let v = serde_json::to_value(&s).unwrap();
        for k in [
            "ownFolder",
            "pageUrl",
            "autoRefresh",
            "needsRefresh",
            "finalUrl",
            "filenameLocked",
            "rangeOk",
            "lastModified",
            "createdAt",
            "completedAt",
            "startAt",
        ] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
        assert!(
            v.get("retryAt").is_none(),
            "optional bookkeeping stays absent until set"
        );
    }

    #[test]
    fn downloaded_sums_the_segments() {
        let mut s = TaskState::default();
        assert_eq!(s.downloaded(), 0);
        s.segments = Some(vec![
            Segment {
                start: 0,
                end: 99,
                pos: 50,
            },
            Segment {
                start: 100,
                end: 199,
                pos: 100,
            },
        ]);
        assert_eq!(s.downloaded(), 50);
    }

    #[test]
    fn unknown_end_is_the_javascript_max_safe_integer() {
        let v = serde_json::to_value(Segment {
            start: 0,
            end: UNKNOWN_END,
            pos: 0,
        })
        .unwrap();
        assert_eq!(v["end"], json!(9007199254740991u64));
    }
}
