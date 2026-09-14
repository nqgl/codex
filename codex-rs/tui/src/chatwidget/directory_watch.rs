//! Bridges directory watcher batches into ordinary queued Codex turns.

use super::*;
use crate::directory_watch::DirectoryWatchNotification;
use crate::directory_watch::DirectoryWatchUrgency;
use crate::directory_watch::WATCH_NOTIFICATION_PREFIX;

const MAX_QUEUED_WATCH_MESSAGE_CHARS: usize = 12_000;

impl ChatWidget {
    pub(crate) fn handle_directory_watch_notification(
        &mut self,
        notification: DirectoryWatchNotification,
    ) {
        let message = notification.user_message();
        match notification.urgency {
            DirectoryWatchUrgency::High => {
                self.submit_user_message(UserMessage::from(message));
            }
            DirectoryWatchUrgency::Low => {
                if let Some(pending) =
                    self.input_queue
                        .queued_user_messages
                        .iter_mut()
                        .find(|pending| {
                            pending
                                .user_message
                                .text
                                .starts_with(WATCH_NOTIFICATION_PREFIX)
                        })
                {
                    pending.user_message.text =
                        merge_watch_messages(&pending.user_message.text, &message);
                    self.refresh_pending_input_preview();
                    return;
                }
                self.queue_user_message(UserMessage::from(message));
            }
        }
    }
}

pub(super) fn merge_watch_messages(previous: &str, current: &str) -> String {
    let combined = format!("{previous}\n\n{current}");
    if combined.chars().count() <= MAX_QUEUED_WATCH_MESSAGE_CHARS {
        combined
    } else {
        format!("{current}\n\n[Earlier queued directory watcher events were omitted.]")
    }
}
