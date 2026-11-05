//! What happens when the last download is over (`afterall.js`).

use crate::state::{Snapshot, Status};

pub const ACTIONS: [&str; 4] = ["none", "quit", "sleep", "shutdown"];

/// True when nothing is running or waiting to run (paused items are the user's choice).
pub fn is_idle(tasks: &[Snapshot]) -> bool {
    !tasks.iter().any(|t| {
        matches!(
            t.state.status,
            Status::Downloading | Status::Queued | Status::Scheduled
        )
    })
}

/// A system command with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub cmd: &'static str,
    pub args: Vec<&'static str>,
}

/// The system command behind an end-of-queue action; `quit` is handled by the app itself.
pub fn command_for(action: &str) -> Option<Command> {
    match action {
        "sleep" => Some(Command {
            cmd: "rundll32.exe",
            args: vec!["powrprof.dll,SetSuspendState", "0", "1", "0"],
        }),
        "shutdown" => Some(Command {
            cmd: "shutdown",
            args: vec![
                "/s",
                "/t",
                "60",
                "/c",
                "Tachy Download : téléchargements terminés",
            ],
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TaskState;

    fn t(status: Status) -> Snapshot {
        Snapshot {
            state: TaskState {
                status,
                ..TaskState::default()
            },
            downloaded: 0,
            speed: 0.0,
            eta: None,
            more: Default::default(),
        }
    }

    #[test]
    fn is_idle_ignores_finished_failed_and_paused_items_only() {
        assert!(is_idle(&[
            t(Status::Completed),
            t(Status::Error),
            t(Status::Paused)
        ]));
        for s in [Status::Downloading, Status::Queued, Status::Scheduled] {
            assert!(!is_idle(&[t(Status::Completed), t(s)]));
        }
        assert!(is_idle(&[]));
    }

    #[test]
    fn command_for_builds_the_system_commands_quit_is_handled_by_the_app() {
        assert_eq!(command_for("quit"), None);
        assert_eq!(command_for("none"), None);
        assert_eq!(
            command_for("shutdown").unwrap().args[..3],
            ["/s", "/t", "60"]
        );
        assert_eq!(command_for("sleep").unwrap().cmd, "rundll32.exe");
    }
}
