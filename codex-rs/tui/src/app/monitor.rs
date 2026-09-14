//! App-level lifecycle handling for user-controlled command monitors.

use super::*;
use crate::monitor::MONITOR_USAGE;
use crate::monitor::MonitorCommand;
use crate::monitor::MonitorExit;
use crate::monitor::MonitorHandle;
use crate::monitor::MonitorNotification;
use crate::monitor::MonitorState;
use crate::monitor::status_message;

impl App {
    pub(super) async fn handle_monitor_command(&mut self, command: MonitorCommand) {
        match command {
            MonitorCommand::Add(request) => {
                if self
                    .monitors
                    .iter()
                    .any(|monitor| monitor.name() == request.name)
                {
                    self.chat_widget.add_error_message(format!(
                        "A monitor named '{}' already exists.",
                        request.name
                    ));
                    return;
                }
                match MonitorHandle::start(request, self.app_event_tx.clone()) {
                    Ok(monitor) => {
                        let status = monitor.status();
                        self.chat_widget.add_info_message(
                            format!(
                                "Started monitor '{}'. Output from `{}` will notify Codex.",
                                status.name, status.command
                            ),
                            Some(format!(
                                "Run /monitor pause {} to pause it, or /monitor remove {} to remove it.",
                                status.name, status.name
                            )),
                        );
                        self.monitors.push(monitor);
                        self.persist_running_monitor(status.name.as_str()).await;
                    }
                    Err(err) => self.chat_widget.add_error_message(err),
                }
            }
            MonitorCommand::List => {
                let statuses = self
                    .monitors
                    .iter()
                    .map(MonitorHandle::status)
                    .collect::<Vec<_>>();
                let (message, hint) = status_message(&statuses);
                self.chat_widget.add_info_message(message, hint);
            }
            MonitorCommand::Pause(name) => {
                let Some(monitor) = self
                    .monitors
                    .iter_mut()
                    .find(|monitor| monitor.name() == name)
                else {
                    self.monitor_not_found(&name);
                    return;
                };
                if monitor.state() == MonitorState::Paused {
                    self.chat_widget
                        .add_info_message(format!("Monitor '{name}' is already paused."), None);
                } else {
                    monitor.pause();
                    self.chat_widget
                        .add_info_message(format!("Paused monitor '{name}'."), None);
                    self.persist_monitor(name.as_str()).await;
                }
            }
            MonitorCommand::Resume(name) => {
                let Some(monitor) = self
                    .monitors
                    .iter_mut()
                    .find(|monitor| monitor.name() == name)
                else {
                    self.monitor_not_found(&name);
                    return;
                };
                if monitor.state() == MonitorState::Running {
                    self.chat_widget
                        .add_info_message(format!("Monitor '{name}' is already running."), None);
                    return;
                }
                match monitor.resume(self.app_event_tx.clone()) {
                    Ok(()) => {
                        self.chat_widget
                            .add_info_message(format!("Resumed monitor '{name}'."), None);
                        self.persist_running_monitor(name.as_str()).await;
                    }
                    Err(err) => self.chat_widget.add_error_message(err),
                }
            }
            MonitorCommand::Remove(name) => {
                let Some(index) = self
                    .monitors
                    .iter()
                    .position(|monitor| monitor.name() == name)
                else {
                    self.monitor_not_found(&name);
                    return;
                };
                self.monitors.remove(index);
                self.chat_widget
                    .add_info_message(format!("Removed monitor '{name}'."), None);
                self.forget_persisted_monitor(name.as_str()).await;
            }
        }
    }

    pub(super) fn handle_monitor_output(&mut self, notification: MonitorNotification) {
        if self
            .monitors
            .iter()
            .any(|monitor| monitor.id() == notification.monitor_id)
        {
            self.chat_widget.handle_monitor_notification(notification);
        }
    }

    pub(super) async fn handle_monitor_exit(&mut self, exit: MonitorExit) {
        let Some(monitor) = self
            .monitors
            .iter_mut()
            .find(|monitor| monitor.id() == exit.monitor_id)
        else {
            return;
        };
        monitor.mark_exited(exit.monitor_id);
        self.forget_persisted_monitor(exit.name.as_str()).await;
        match exit.result {
            Ok(Some(0)) => self.chat_widget.add_info_message(
                format!("Monitor '{}' exited normally.", exit.name),
                Some(format!("/monitor resume {}", exit.name)),
            ),
            Ok(code) => self.chat_widget.add_error_message(format!(
                "Monitor '{}' exited with status {}.",
                exit.name,
                code.map_or_else(|| "unknown".to_string(), |code| code.to_string())
            )),
            Err(err) => self
                .chat_widget
                .add_error_message(format!("Monitor '{}' failed: {err}", exit.name)),
        }
    }

    fn monitor_not_found(&mut self, name: &str) {
        self.chat_widget.add_error_message(format!(
            "No monitor named '{name}' exists.\n{MONITOR_USAGE}"
        ));
    }

    pub(super) async fn restore_thread_monitors(&mut self, thread_id: ThreadId) {
        self.monitors.clear();
        let Some(state_db) = self.state_db.as_ref() else {
            return;
        };
        let persisted = match state_db.list_thread_monitors(thread_id).await {
            Ok(persisted) => persisted,
            Err(err) => {
                tracing::warn!(%thread_id, "failed to load persisted monitors: {err}");
                return;
            }
        };
        for monitor in persisted {
            let name = monitor.name.clone();
            match MonitorHandle::from_persisted(monitor, self.app_event_tx.clone()) {
                Ok(monitor) => self.monitors.push(monitor),
                Err(err) => tracing::warn!(%thread_id, %name, "failed to restart monitor: {err}"),
            }
        }
    }

    async fn persist_running_monitor(&mut self, name: &str) {
        self.persist_monitor(name).await;
    }

    async fn persist_monitor(&mut self, name: &str) {
        let Some(thread_id) = self.primary_thread_id else {
            return;
        };
        let Some(state_db) = self.state_db.as_ref() else {
            return;
        };
        let Some(monitor) = self
            .monitors
            .iter()
            .find(|monitor| monitor.name() == name)
            .map(MonitorHandle::persisted)
        else {
            return;
        };
        if let Err(err) = state_db.upsert_thread_monitor(thread_id, &monitor).await {
            tracing::warn!(%thread_id, %name, "failed to persist monitor: {err}");
        }
    }

    pub(super) async fn persist_all_monitors(&self) {
        let Some(thread_id) = self.primary_thread_id else {
            return;
        };
        let Some(state_db) = self.state_db.as_ref() else {
            return;
        };
        for monitor in &self.monitors {
            let name = monitor.name();
            let monitor = monitor.persisted();
            if let Err(err) = state_db.upsert_thread_monitor(thread_id, &monitor).await {
                tracing::warn!(%thread_id, %name, "failed to persist monitor: {err}");
            }
        }
    }

    pub(super) fn pause_monitors_for_fork(&mut self) {
        self.monitors
            .retain(|monitor| monitor.state() != MonitorState::Exited);
        for monitor in &mut self.monitors {
            monitor.pause();
        }
    }

    pub(super) async fn inherit_thread_monitors_paused(
        &mut self,
        source_thread_id: ThreadId,
        fork_thread_id: ThreadId,
    ) {
        self.monitors.clear();
        let Some(state_db) = self.state_db.as_ref() else {
            return;
        };
        let inherited = match state_db.list_thread_monitors(source_thread_id).await {
            Ok(inherited) => inherited,
            Err(err) => {
                tracing::warn!(%source_thread_id, "failed to load monitors for fork: {err}");
                return;
            }
        };
        for mut monitor in inherited {
            monitor.running = false;
            if let Err(err) = state_db
                .upsert_thread_monitor(fork_thread_id, &monitor)
                .await
            {
                tracing::warn!(%fork_thread_id, name = %monitor.name, "failed to persist fork monitor: {err}");
                continue;
            }
            self.monitors.push(MonitorHandle::paused(monitor.into()));
        }
    }

    async fn forget_persisted_monitor(&mut self, name: &str) {
        let Some(thread_id) = self.primary_thread_id else {
            return;
        };
        let Some(state_db) = self.state_db.as_ref() else {
            return;
        };
        if let Err(err) = state_db.delete_thread_monitor(thread_id, name).await {
            tracing::warn!(%thread_id, %name, "failed to remove persisted monitor: {err}");
        }
    }
}

#[cfg(test)]
#[path = "monitor_tests.rs"]
mod tests;
