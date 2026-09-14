//! User-visible watcher start and status messages.

use super::DirectoryWatchBackend;
use super::DirectoryWatchMode;
use super::DirectoryWatchStatus;
use super::DirectoryWatchTrust;
use super::WATCH_USAGE;
use super::display_path;

impl DirectoryWatchStatus {
    pub(crate) fn same_configuration(&self, other: &Self) -> bool {
        self.root == other.root
            && self.filter == other.filter
            && self.mode == other.mode
            && self.trust == other.trust
    }
}

pub(crate) fn started_message(status: &DirectoryWatchStatus) -> (String, String) {
    let target = match (status.mode, status.watches_git_commits) {
        (DirectoryWatchMode::FilesAndCommits, true) => format!(
            "{} and {}",
            status.filter.file_description(),
            status.filter.commit_description()
        ),
        (DirectoryWatchMode::FilesAndCommits, false) => {
            format!(
                "{}; no Git repository was found",
                status.filter.file_description()
            )
        }
        (DirectoryWatchMode::FilesOnly, true) | (DirectoryWatchMode::FilesOnly, false) => {
            status.filter.file_description()
        }
        (DirectoryWatchMode::CommitsOnly, true) => {
            format!("only {}", status.filter.commit_description())
        }
        (DirectoryWatchMode::CommitsOnly, false) => status.filter.commit_description(),
    };
    let polling = if status.backend == DirectoryWatchBackend::Polling {
        " Native file notifications were unavailable, so changes are checked every 2 seconds."
    } else {
        ""
    };
    let trust = if status.trust == DirectoryWatchTrust::Trusted {
        " Watcher notifications are trusted."
    } else {
        ""
    };
    (
        format!("Watching {}", display_path(&status.root)),
        format!(
            "Monitoring {target} recursively.{polling}{trust} Changes notify Codex; use /watch stop to stop."
        ),
    )
}

pub(crate) fn status_message(statuses: &[&DirectoryWatchStatus]) -> (String, Option<String>) {
    match statuses {
        [] => (
            "No directory watcher is running.".to_string(),
            Some(WATCH_USAGE.to_string()),
        ),
        [status] => {
            let (message, hint) = started_message(status);
            (message, Some(hint))
        }
        statuses => {
            let hint = statuses
                .iter()
                .map(|status| {
                    let (message, hint) = started_message(status);
                    format!("- {message}: {hint}")
                })
                .collect::<Vec<_>>()
                .join("\n");
            (
                format!("{} directory watchers are running.", statuses.len()),
                Some(hint),
            )
        }
    }
}
