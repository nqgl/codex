use super::App;
use crate::group::GroupCommand;
use codex_group_mail_extension::GroupMailStore;

impl App {
    pub(super) async fn handle_group_command(&mut self, command: GroupCommand) {
        if self.app_server_target.uses_remote_workspace() {
            self.chat_widget
                .add_error_message("/group currently requires a local Codex server.".to_string());
            return;
        }
        let Some(thread_id) = self.primary_thread_id else {
            self.chat_widget
                .add_error_message("Start or resume a session before using /group.".to_string());
            return;
        };
        if self.state_db.is_none() {
            self.chat_widget.add_error_message(
                "Group mail requires persistent local session state.".to_string(),
            );
            return;
        }
        let store = match GroupMailStore::open(self.config.sqlite_config().home()).await {
            Ok(store) => store,
            Err(error) => {
                self.chat_widget
                    .add_error_message(format!("Could not open group mail: {error}"));
                return;
            }
        };
        let result = match command {
            GroupCommand::Status => match store.membership(thread_id).await {
                Ok(Some(member)) => match store.members(thread_id).await {
                    Ok(peers) => {
                        let peers = peers
                            .into_iter()
                            .map(|peer| {
                                format!(
                                    "{} ({})",
                                    peer.name,
                                    if peer.online { "online" } else { "offline" }
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        Ok(format!(
                            "Group: {}. You are {}. Members: {peers}",
                            member.group, member.name
                        ))
                    }
                    Err(error) => Err(error),
                },
                Ok(None) => Ok(
                    "This session is not in a group. Use /group join <group> <name>.".to_string(),
                ),
                Err(error) => Err(error),
            },
            GroupCommand::Join { group, name } => store
                .join(thread_id, &group, &name)
                .await
                .map(|()| format!("Joined {group} as {name}.")),
            GroupCommand::Leave => store.leave(thread_id).await.map(|removed| {
                if removed {
                    "Left the group.".to_string()
                } else {
                    "This session is not in a group.".to_string()
                }
            }),
        };
        match result {
            Ok(message) => self.chat_widget.add_info_message(message, None),
            Err(error) => self.chat_widget.add_error_message(error.to_string()),
        }
    }
}
