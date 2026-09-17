//! Retract buffered input without interrupting a task or rewriting conversation history.

use super::*;
use codex_app_server_protocol::TurnSteerCancelParams;
use codex_app_server_protocol::TurnSteerCancelResponse;

impl TurnRequestProcessor {
    pub(crate) async fn turn_steer_cancel(
        &self,
        request_id: &ConnectionRequestId,
        params: TurnSteerCancelParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        if params.expected_turn_id.is_empty() || params.client_user_message_id.is_empty() {
            return Err(invalid_request(
                "expectedTurnId and clientUserMessageId must not be empty",
            ));
        }
        let (_, thread) = self.load_thread(&params.thread_id).await?;
        self.ensure_direct_input_allowed(request_id, thread.as_ref())
            .await?;
        let cancelled = thread
            .cancel_pending_user_input(&params.expected_turn_id, &params.client_user_message_id)
            .await;
        Ok(Some(TurnSteerCancelResponse { cancelled }.into()))
    }
}
