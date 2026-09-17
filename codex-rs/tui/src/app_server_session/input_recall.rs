//! Server-confirmed cancellation of an unconsumed steer.

use super::AppServerSession;
use codex_app_server_client::TypedRequestError;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::TurnSteerCancelParams;
use codex_app_server_protocol::TurnSteerCancelResponse;

impl AppServerSession {
    pub(crate) async fn cancel_pending_steer(
        &mut self,
        params: TurnSteerCancelParams,
    ) -> Result<TurnSteerCancelResponse, TypedRequestError> {
        let request_id = self.next_request_id();
        self.client
            .request_typed(ClientRequest::TurnSteerCancel { request_id, params })
            .await
    }
}
