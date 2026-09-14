//! App-level lifecycle handling for the user-controlled directory watcher.

use super::*;
use crate::directory_watch::DirectoryWatchCommand;
use crate::directory_watch::DirectoryWatchHandle;
use crate::directory_watch::DirectoryWatchNotification;
use crate::directory_watch::WATCH_USAGE;
use crate::directory_watch::started_message;
use crate::directory_watch::status_message;

impl App {
    pub(super) async fn handle_directory_watch_command(&mut self, command: DirectoryWatchCommand) {
        match command {
            DirectoryWatchCommand::Start(request) => {
                match DirectoryWatchHandle::start(request, self.app_event_tx.clone()).await {
                    Ok(watcher) => {
                        let (message, hint) = started_message(watcher.status());
                        let duplicate = self
                            .directory_watches
                            .iter()
                            .any(|existing| existing.status().same_configuration(watcher.status()));
                        if duplicate {
                            self.chat_widget.add_info_message(
                                "That directory watcher is already running.".to_string(),
                                Some(hint),
                            );
                        } else {
                            self.directory_watches.push(watcher);
                            self.chat_widget.add_info_message(message, Some(hint));
                        }
                    }
                    Err(err) => self.chat_widget.add_error_message(err),
                }
            }
            DirectoryWatchCommand::Status => {
                let status = self
                    .directory_watches
                    .iter()
                    .map(DirectoryWatchHandle::status)
                    .collect::<Vec<_>>();
                let (message, hint) = status_message(&status);
                self.chat_widget.add_info_message(message, hint);
            }
            DirectoryWatchCommand::StopAll => {
                if self.directory_watches.is_empty() {
                    let (message, hint) = status_message(&[]);
                    self.chat_widget.add_info_message(message, hint);
                } else {
                    let count = self.directory_watches.len();
                    self.directory_watches.clear();
                    self.chat_widget.add_info_message(
                        format!("Stopped {count} directory watcher(s)."),
                        /*hint*/ None,
                    );
                }
            }
            DirectoryWatchCommand::StopRoot(root) => {
                let root = dunce::canonicalize(&root).unwrap_or(root);
                let previous_len = self.directory_watches.len();
                self.directory_watches
                    .retain(|watcher| watcher.status().root != root);
                let stopped = previous_len - self.directory_watches.len();
                if stopped > 0 {
                    self.chat_widget.add_info_message(
                        format!("Stopped {stopped} watcher(s) for {}.", root.display()),
                        /*hint*/ None,
                    );
                } else {
                    self.chat_widget.add_info_message(
                        format!("No watcher is running for {}.", root.display()),
                        Some(WATCH_USAGE.to_string()),
                    );
                }
            }
        }
    }

    pub(super) fn handle_directory_watch_notification(
        &mut self,
        notification: DirectoryWatchNotification,
    ) {
        let is_current = self
            .directory_watches
            .iter()
            .any(|watcher| watcher.id() == notification.watch_id);
        if is_current {
            self.chat_widget
                .handle_directory_watch_notification(notification);
        }
    }
}
