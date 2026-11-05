//! Settings (`DEFAULT_SETTINGS` and `_normalize` in `manager.js`, annex A of the design).
//!
//! The settings travel as JSON (the file, the window and the extension all speak it): a patch is merged
//! shallowly over the current ones, as `{ ...settings, ...patch }` does, then normalised, so unknown keys are
//! dropped and every value lands in its range.

use crate::net::normalize_proxy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedSchedule {
    pub enabled: bool,
    pub from: String,
    pub to: String,
    #[serde(rename = "limitKBps")]
    pub limit_kbps: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub enabled: bool,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub dir: String,
    pub connections: u32,
    pub max_concurrent: u32,
    #[serde(rename = "speedLimitKBps")]
    pub speed_limit_kbps: u64,
    pub video_max_height: u32,
    pub convert_to_mp4: bool,
    pub categories: bool,
    pub auto_retry: bool,
    pub adaptive: bool,
    pub proxy: String,
    pub speed_schedule: SpeedSchedule,
    pub rules: String,
    pub duplicates: bool,
    pub auto_extract: bool,
    pub auto_start: bool,
    pub close_to_tray: bool,
    pub notifications: bool,
    pub confirm_downloads: bool,
    pub auto_update: bool,
    pub yt_cookies: String,
    pub after_all: String,
    pub schedule: Schedule,
}

/// `DEFAULT_SETTINGS` of `manager.js`.
fn defaults_json() -> Value {
    json!({
        "dir": "", "connections": 8, "maxConcurrent": 3, "speedLimitKBps": 0, "videoMaxHeight": 1080, "convertToMp4": true,
        "categories": false, "autoRetry": true, "adaptive": true, "proxy": "",
        "speedSchedule": { "enabled": false, "from": "08:00", "to": "18:00", "limitKBps": 500 },
        "rules": "", "duplicates": true, "autoExtract": false, "autoStart": false, "closeToTray": true, "notifications": true,
        "confirmDownloads": true, "autoUpdate": true, "ytCookies": "", "afterAll": "none",
        "schedule": { "enabled": false, "from": "01:00", "to": "07:00" },
    })
}

impl Default for Settings {
    fn default() -> Self {
        Settings::normalize(&defaults_json())
    }
}

/// `Boolean(v)` in JavaScript.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `v !== false`.
fn not_false(v: Option<&Value>) -> bool {
    !matches!(v, Some(Value::Bool(false)))
}

/// `Number(v)` in JavaScript (NaN for what cannot be read).
fn number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        Some(_) => f64::NAN,
    }
}

/// `Math.round(Number(v))`, rounding halves up like JavaScript.
fn round_number(v: Option<&Value>) -> f64 {
    let n = number(v);
    if n.is_nan() { 0.0 } else { (n + 0.5).floor() }
}

/// `clamp` of `manager.js`: `Math.min(hi, Math.max(lo, Math.round(Number(n)) || lo))`.
fn clamp(v: Option<&Value>, lo: f64, hi: f64) -> f64 {
    let r = round_number(v);
    hi.min(lo.max(if r == 0.0 { lo } else { r }))
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

/// "HH:MM" as minutes since midnight, or `None`.
pub fn to_minutes(hhmm: &str) -> Option<u32> {
    let (h, m) = hhmm.split_once(':')?;
    if h.is_empty()
        || h.len() > 2
        || m.len() != 2
        || !h.bytes().all(|b| b.is_ascii_digit())
        || !m.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// True when `minutes` (since midnight) falls inside the daily window; windows may cross midnight.
pub fn in_window(enabled: bool, from: &str, to: &str, minutes_now: u32) -> bool {
    if !enabled {
        return true;
    }
    let (Some(from), Some(to)) = (to_minutes(from), to_minutes(to)) else {
        return true;
    };
    if from == to {
        return true;
    }
    if from < to {
        minutes_now >= from && minutes_now < to
    } else {
        minutes_now >= from || minutes_now < to
    }
}

fn valid_time(v: Option<&Value>, default: &str) -> String {
    match v {
        Some(Value::String(s)) if to_minutes(s).is_some() => s.clone(),
        _ => default.to_string(),
    }
}

impl Settings {
    /// `_normalize`: every key gets a valid value, unknown keys are dropped.
    pub fn normalize(s: &Value) -> Settings {
        let get = |k: &str| s.get(k);
        let sch = s.get("speedSchedule");
        let sch_get = |k: &str| sch.and_then(|v| v.get(k));
        let sched = s.get("schedule");
        let sched_get = |k: &str| sched.and_then(|v| v.get(k));
        let height = number(get("videoMaxHeight"));
        let in_list = |v: Option<&Value>, list: &[&str]| -> String {
            match v {
                Some(Value::String(x)) if list.contains(&x.as_str()) => x.clone(),
                _ => String::new(),
            }
        };
        let after_all = in_list(get("afterAll"), &["none", "quit", "sleep", "shutdown"]);
        Settings {
            dir: if truthy(get("dir")) {
                text(get("dir"))
            } else {
                String::new()
            },
            connections: clamp(get("connections"), 1.0, 32.0) as u32,
            max_concurrent: clamp(get("maxConcurrent"), 1.0, 10.0) as u32,
            speed_limit_kbps: round_number(get("speedLimitKBps")).max(0.0) as u64,
            video_max_height: if [360.0, 480.0, 720.0, 1080.0, 1440.0, 2160.0].contains(&height) {
                height as u32
            } else {
                1080
            },
            convert_to_mp4: not_false(get("convertToMp4")),
            categories: truthy(get("categories")),
            auto_retry: not_false(get("autoRetry")),
            adaptive: not_false(get("adaptive")),
            proxy: normalize_proxy(&text(get("proxy"))).unwrap_or_default(),
            speed_schedule: SpeedSchedule {
                enabled: truthy(sch_get("enabled")),
                from: valid_time(sch_get("from"), "08:00"),
                to: valid_time(sch_get("to"), "18:00"),
                limit_kbps: round_number(sch_get("limitKBps")).max(0.0) as u64,
            },
            rules: text(get("rules")).chars().take(4000).collect(),
            duplicates: not_false(get("duplicates")),
            auto_extract: truthy(get("autoExtract")),
            auto_start: truthy(get("autoStart")),
            close_to_tray: not_false(get("closeToTray")),
            notifications: not_false(get("notifications")),
            confirm_downloads: not_false(get("confirmDownloads")),
            auto_update: not_false(get("autoUpdate")),
            yt_cookies: in_list(get("ytCookies"), &["firefox", "edge", "chrome", "brave"]),
            after_all: if after_all.is_empty() {
                "none".into()
            } else {
                after_all
            },
            schedule: Schedule {
                enabled: truthy(sched_get("enabled")),
                from: valid_time(sched_get("from"), "01:00"),
                to: valid_time(sched_get("to"), "07:00"),
            },
        }
    }

    /// `{ ...DEFAULT_SETTINGS, ...patch }` normalised: how the manager starts from the settings it is given.
    pub fn from_patch(patch: &Value) -> Settings {
        Settings::default().merged(patch)
    }

    /// `{ ...this, ...patch }` normalised again.
    pub fn merged(&self, patch: &Value) -> Settings {
        let mut base = serde_json::to_value(self).unwrap_or_else(|_| json!({}));
        if let (Some(b), Some(p)) = (base.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                b.insert(k.clone(), v.clone());
            }
        }
        Settings::normalize(&base)
    }

    /// The limit in force (KB/s): the fixed one, tightened by the time-of-day one while its window is open.
    /// `minutes_now` is the local time since midnight.
    pub fn effective_limit(&self, minutes_now: u32) -> u64 {
        let sch = &self.speed_schedule;
        let mut caps = vec![self.speed_limit_kbps];
        if sch.enabled && sch.limit_kbps > 0 && in_window(true, &sch.from, &sch.to, minutes_now) {
            caps.push(sch.limit_kbps);
        }
        caps.into_iter().filter(|v| *v > 0).min().unwrap_or(0)
    }

    pub fn in_schedule_window(&self, minutes_now: u32) -> bool {
        in_window(
            self.schedule.enabled,
            &self.schedule.from,
            &self.schedule.to,
            minutes_now,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_ones() {
        let s = Settings::default();
        assert_eq!(s.connections, 8);
        assert_eq!(s.max_concurrent, 3);
        assert_eq!(s.video_max_height, 1080);
        assert!(
            s.convert_to_mp4
                && s.auto_retry
                && s.adaptive
                && s.duplicates
                && s.close_to_tray
                && s.notifications
                && s.confirm_downloads
                && s.auto_update
        );
        assert!(!s.categories && !s.auto_extract && !s.auto_start);
        assert_eq!(
            s.speed_schedule,
            SpeedSchedule {
                enabled: false,
                from: "08:00".into(),
                to: "18:00".into(),
                limit_kbps: 500
            }
        );
        assert_eq!(
            s.schedule,
            Schedule {
                enabled: false,
                from: "01:00".into(),
                to: "07:00".into()
            }
        );
        assert_eq!(s.after_all, "none");
        assert_eq!(s.dir, "");
    }

    #[test]
    fn values_land_in_their_ranges() {
        let s = Settings::from_patch(
            &json!({ "connections": 99, "maxConcurrent": 0, "speedLimitKBps": -5, "videoMaxHeight": 999, "ytCookies": "opera", "afterAll": "reboot", "rules": "x".repeat(5000) }),
        );
        assert_eq!(s.connections, 32);
        assert_eq!(s.max_concurrent, 1);
        assert_eq!(s.speed_limit_kbps, 0);
        assert_eq!(s.video_max_height, 1080);
        assert_eq!(s.yt_cookies, "");
        assert_eq!(s.after_all, "none");
        assert_eq!(s.rules.len(), 4000);
        let s = Settings::from_patch(
            &json!({ "connections": "12", "videoMaxHeight": 720, "ytCookies": "firefox", "afterAll": "quit", "connections2": 1 }),
        );
        assert_eq!(
            (
                s.connections,
                s.video_max_height,
                s.yt_cookies.as_str(),
                s.after_all.as_str()
            ),
            (12, 720, "firefox", "quit")
        );
        assert_eq!(
            Settings::from_patch(&json!({ "connections": 0 })).connections,
            1,
            "0 is not a valid count: the minimum applies"
        );
        assert_eq!(
            Settings::from_patch(&json!({ "connections": 2.5 })).connections,
            3,
            "halves round up like Math.round"
        );
    }

    #[test]
    fn booleans_follow_their_javascript_rule() {
        let s = Settings::from_patch(
            &json!({ "convertToMp4": 0, "autoRetry": false, "categories": "yes", "autoExtract": 1, "closeToTray": null }),
        );
        assert!(
            s.convert_to_mp4,
            "only `false` turns a default-true flag off"
        );
        assert!(!s.auto_retry);
        assert!(s.categories && s.auto_extract);
        assert!(s.close_to_tray);
    }

    #[test]
    fn bad_times_and_proxy_fall_back() {
        let s = Settings::from_patch(
            &json!({ "speedSchedule": { "enabled": true, "from": "25:00", "to": "7:30", "limitKBps": 200 }, "schedule": { "from": "bad" }, "proxy": "ftp://x" }),
        );
        assert_eq!(s.speed_schedule.from, "08:00");
        assert_eq!(s.speed_schedule.to, "7:30", "kept as typed when valid");
        assert_eq!(s.speed_schedule.limit_kbps, 200);
        assert_eq!(s.schedule.from, "01:00");
        assert_eq!(s.proxy, "");
        assert_eq!(
            Settings::from_patch(&json!({ "proxy": "proxy.local:3128" })).proxy,
            "http://proxy.local:3128"
        );
    }

    #[test]
    fn merged_keeps_the_rest_and_replaces_nested_objects_whole() {
        let base = Settings::from_patch(
            &json!({ "connections": 4, "speedSchedule": { "enabled": true, "from": "09:00", "to": "10:00", "limitKBps": 100 } }),
        );
        let next =
            base.merged(&json!({ "maxConcurrent": 5, "speedSchedule": { "enabled": true } }));
        assert_eq!(next.connections, 4);
        assert_eq!(next.max_concurrent, 5);
        assert_eq!(
            next.speed_schedule.from, "08:00",
            "the nested object is replaced, not merged"
        );
    }

    #[test]
    fn windows_cross_midnight() {
        assert!(in_window(true, "22:00", "06:00", 23 * 60 + 30));
        assert!(in_window(true, "22:00", "06:00", 5 * 60 + 59));
        assert!(!in_window(true, "22:00", "06:00", 12 * 60));
        assert!(in_window(false, "22:00", "06:00", 12 * 60));
        assert!(
            in_window(true, "08:00", "08:00", 3),
            "an empty window means always"
        );
    }

    #[test]
    fn the_limit_in_force_is_the_tighter_one() {
        let mut s = Settings::from_patch(
            &json!({ "speedLimitKBps": 0, "speedSchedule": { "enabled": true, "from": "08:00", "to": "18:00", "limitKBps": 200 } }),
        );
        let at = |h: u32| h * 60;
        assert_eq!(s.effective_limit(at(10)), 200);
        assert_eq!(s.effective_limit(at(20)), 0);
        s.speed_limit_kbps = 100;
        assert_eq!(
            (s.effective_limit(at(10)), s.effective_limit(at(20))),
            (100, 100)
        );
        s.speed_limit_kbps = 500;
        assert_eq!(s.effective_limit(at(10)), 200);
        s.speed_schedule.enabled = false;
        assert_eq!(s.effective_limit(at(10)), 500);
        s = s.merged(&json!({ "speedLimitKBps": 0, "speedSchedule": { "enabled": true, "from": "22:00", "to": "06:00", "limitKBps": 50 } }));
        assert_eq!(
            (
                s.effective_limit(at(23)),
                s.effective_limit(at(3)),
                s.effective_limit(at(12))
            ),
            (50, 50, 0)
        );
    }

    #[test]
    fn round_trips_through_json_with_the_documented_keys() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        for k in [
            "speedLimitKBps",
            "maxConcurrent",
            "videoMaxHeight",
            "convertToMp4",
            "autoRetry",
            "closeToTray",
            "confirmDownloads",
            "autoUpdate",
            "ytCookies",
            "afterAll",
            "speedSchedule",
            "schedule",
        ] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
        assert!(v["speedSchedule"].get("limitKBps").is_some());
        assert_eq!(Settings::normalize(&v), Settings::default());
    }
}
