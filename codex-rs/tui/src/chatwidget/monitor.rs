//! Bridges monitor output into ordinary Codex turns.

use super::*;
use crate::monitor::MonitorNotification;

impl ChatWidget {
    pub(crate) fn handle_monitor_notification(&mut self, notification: MonitorNotification) {
        self.submit_user_message(UserMessage::from(notification.user_message()));
    }
}
