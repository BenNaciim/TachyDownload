//! Overall progress of what is being downloaded, for the taskbar button (`progress.js`).

use crate::state::{Snapshot, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nothing is running.
    None,
    /// Something runs but no size is known.
    Indeterminate,
    /// A fraction between 0 and 1.
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overall {
    pub mode: Mode,
    /// 0 to 1 in `Normal` mode; -1 for `None` and 2 for `Indeterminate`, as the JavaScript engine handed them out.
    pub value: f64,
}

/// The weight of each running file follows its size; files without a known size and those that wait are left out,
/// so the bar stays honest about what is known.
pub fn overall_progress(tasks: &[Snapshot]) -> Overall {
    let running: Vec<&Snapshot> = tasks
        .iter()
        .filter(|t| t.state.status == Status::Downloading)
        .collect();
    if running.is_empty() {
        return Overall {
            mode: Mode::None,
            value: -1.0,
        };
    }
    let (mut total, mut got) = (0u64, 0u64);
    for t in running {
        if let Some(size) = t.state.size.filter(|s| *s > 0) {
            total += size;
            got += t.downloaded.min(size);
        }
    }
    if total == 0 {
        return Overall {
            mode: Mode::Indeterminate,
            value: 2.0,
        };
    }
    Overall {
        mode: Mode::Normal,
        value: (got as f64 / total as f64).min(1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TaskState;

    fn snap(status: Status, size: Option<u64>, downloaded: u64) -> Snapshot {
        Snapshot {
            state: TaskState {
                status,
                size,
                ..TaskState::default()
            },
            downloaded,
            speed: 0.0,
            eta: None,
            more: Default::default(),
        }
    }

    #[test]
    fn taskbar_progress_nothing_running_unknown_sizes_and_a_real_fraction() {
        let none = overall_progress(&[]);
        assert_eq!((none.mode, none.value), (Mode::None, -1.0));
        assert_eq!(
            overall_progress(&[
                snap(Status::Paused, Some(10), 5),
                snap(Status::Completed, Some(10), 10)
            ])
            .mode,
            Mode::None
        );
        assert_eq!(
            overall_progress(&[snap(Status::Downloading, None, 500)]).mode,
            Mode::Indeterminate
        );
        let p = overall_progress(&[
            snap(Status::Downloading, Some(1000), 250),
            snap(Status::Downloading, Some(3000), 1750),
            snap(Status::Downloading, None, 99999),
            snap(Status::Queued, Some(5000), 0),
        ]);
        assert_eq!(p.mode, Mode::Normal);
        assert_eq!(
            p.value, 0.5,
            "the weight of each file follows its size; unsized and waiting ones are left out"
        );
        assert_eq!(
            overall_progress(&[snap(Status::Downloading, Some(100), 400)]).value,
            1.0,
            "never above 100 %"
        );
    }
}
