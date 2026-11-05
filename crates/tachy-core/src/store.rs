//! `downloads.json`: `{ "settings": {…}, "tasks": [state…] }`, written atomically (a temporary file, then a
//! rename). A file that cannot be read is copied aside as `downloads.json.illisible` before the next save
//! overwrites it, and the manager starts empty.

use crate::settings::Settings;
use crate::state::TaskState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct Loaded {
    pub settings: Option<Value>,
    pub tasks: Vec<TaskState>,
}

#[derive(Deserialize)]
struct Raw {
    settings: Option<Value>,
    #[serde(default)]
    tasks: Vec<Value>,
}

/// `None` when there is no file, or when it cannot be read (then it is kept aside).
pub fn load(path: &Path) -> Option<Loaded> {
    let bytes = std::fs::read(path).ok()?;
    let parsed: Option<Raw> = serde_json::from_slice(&bytes).ok();
    let Some(raw) = parsed else {
        let aside = PathBuf::from(format!("{}.illisible", path.display()));
        let _ = std::fs::write(aside, &bytes);
        return None;
    };
    let tasks = raw
        .tasks
        .into_iter()
        .filter_map(|v| serde_json::from_value::<TaskState>(v).ok())
        .collect();
    Some(Loaded {
        settings: raw.settings,
        tasks,
    })
}

#[derive(Serialize)]
struct Payload<'a> {
    settings: &'a Settings,
    tasks: &'a [TaskState],
}

pub fn save(path: &Path, settings: &Settings, tasks: &[TaskState]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, serde_json::to_vec(&Payload { settings, tasks })?)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Status;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("tachy-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn saves_and_loads_without_losing_a_field() {
        let dir = tmp();
        let path = dir.join("downloads.json");
        let mut t = TaskState {
            id: "a".into(),
            status: Status::Paused,
            url: "http://h/x".into(),
            ..TaskState::default()
        };
        t.extra
            .insert("fromTheFuture".into(), serde_json::json!([1, 2]));
        save(&path, &Settings::default(), &[t]).unwrap();
        assert!(
            !dir.join("downloads.json.tmp").exists(),
            "the temporary file is renamed away"
        );
        let back = load(&path).unwrap();
        assert_eq!(back.tasks.len(), 1);
        assert_eq!(back.tasks[0].status, Status::Paused);
        assert_eq!(
            back.tasks[0].extra["fromTheFuture"],
            serde_json::json!([1, 2])
        );
        assert_eq!(back.settings.unwrap()["connections"], 8);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_missing_file_is_nothing_and_a_broken_one_is_kept_aside() {
        let dir = tmp();
        let path = dir.join("downloads.json");
        assert!(load(&path).is_none());
        std::fs::write(&path, b"{ this is not json").unwrap();
        assert!(load(&path).is_none());
        assert_eq!(
            std::fs::read(dir.join("downloads.json.illisible")).unwrap(),
            b"{ this is not json"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
