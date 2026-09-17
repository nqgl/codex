//! Recall buffered input. Enter steers require server confirmation; consumed history is immutable.

use super::*;

impl ChatWidget {
    pub(super) fn handle_pending_input_recall(&mut self, key: KeyEvent) -> bool {
        let empty_up = key.code == KeyCode::Up
            && key.modifiers.is_empty()
            && self.bottom_pane.composer_is_empty();
        if key.kind != KeyEventKind::Press
            || !(empty_up || self.chat_keymap.edit_queued_message.is_pressed(key))
            || !self.bottom_pane.no_modal_or_popup_active()
            || self.blocks_direct_input
        {
            return false;
        }
        if self.input_queue.recalling_steer.is_some() {
            return true;
        }
        let queued_order = self
            .input_queue
            .queued_user_messages
            .back()
            .map(|message| message.recall_order);
        let pending = self.input_queue.pending_steers.back();
        if let Some(pending) =
            pending.filter(|pending| queued_order.is_none_or(|order| pending.recall_order > order))
        {
            let client_id = pending.client_id.clone();
            if let (Some(thread_id), Some(expected_turn_id)) =
                (self.thread_id, self.turn_lifecycle.last_turn_id.clone())
                && self.submit_op(AppCommand::RecallPendingSteer {
                    thread_id,
                    expected_turn_id,
                    client_id: client_id.clone(),
                })
            {
                self.input_queue.recalling_steer = Some(client_id);
            }
            return true;
        }
        if let Some(composer) = self.pop_latest_queued_composer_state() {
            self.restore_composer_state(composer);
            self.refresh_pending_input_preview();
            self.request_redraw();
            return true;
        }
        false
    }

    pub(crate) fn finish_pending_input_recall(
        &mut self,
        client_id: &str,
        result: Result<bool, String>,
    ) {
        if self.input_queue.recalling_steer.as_deref() != Some(client_id) {
            return;
        }
        self.input_queue.recalling_steer = None;
        match result {
            Ok(true) => {
                if let Some(index) = self
                    .input_queue
                    .pending_steers
                    .iter()
                    .position(|message| message.client_id == client_id)
                    && let Some(message) = self.input_queue.pending_steers.remove(index)
                {
                    self.restore_user_message_to_composer(user_message_for_restore(
                        message.user_message,
                        &message.history_record,
                    ));
                }
            }
            Ok(false) => self.add_info_message(
                "That message is no longer pending; it could not be recalled.".to_string(),
                /*hint*/ None,
            ),
            Err(error) => {
                self.add_error_message(format!("Could not recall the pending message: {error}"))
            }
        }
        self.refresh_pending_input_preview();
        self.request_redraw();
    }
}
