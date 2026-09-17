use anyhow::Result;
use app_test_support::TestAppServer;
use app_test_support::write_mock_responses_config_toml_with_chatgpt_base_url;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::TurnSteerCancelParams;
use codex_app_server_protocol::TurnSteerCancelResponse;
use codex_app_server_protocol::TurnSteerParams;
use codex_app_server_protocol::TurnSteerResponse;
use codex_app_server_protocol::UserInput;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::sse;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::start_streaming_sse_server;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::time::timeout;

#[tokio::test]
async fn cancel_pending_steer_removes_only_unconsumed_unique_input_without_interrupting()
-> Result<()> {
    let (first_tx, first_rx) = oneshot::channel();
    let (second_tx, second_rx) = oneshot::channel();
    let (server, _) = start_streaming_sse_server(
        [("first", first_rx), ("second", second_rx)]
            .into_iter()
            .map(|(id, gate)| {
                vec![
                    StreamingSseChunk {
                        gate: None,
                        body: sse(vec![ev_response_created(id)]),
                    },
                    StreamingSseChunk {
                        gate: Some(gate),
                        body: sse(vec![ev_completed(id)]),
                    },
                ]
            })
            .collect(),
    )
    .await;
    let home = TempDir::new()?;
    write_mock_responses_config_toml_with_chatgpt_base_url(
        home.path(),
        server.uri(),
        server.uri(),
    )?;
    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .without_managed_config()
        .build_initialized()
        .await?;
    let thread = app.start_thread(ThreadStartParams::default()).await?.thread;
    let TurnStartResponse { turn } = app
        .request(|request_id| ClientRequest::TurnStart {
            request_id,
            params: TurnStartParams {
                thread_id: thread.id.clone(),
                input: vec![UserInput::Text {
                    text: "initial message".into(),
                    text_elements: Vec::new(),
                }],
                ..Default::default()
            },
        })
        .await?;
    timeout(
        Duration::from_secs(30),
        server.wait_for_request_count(/*count*/ 1),
    )
    .await?;
    for (client_id, text) in [
        ("recall", "recalled message"),
        ("keep", "retained message"),
        ("duplicate", "duplicate one"),
        ("duplicate", "duplicate two"),
    ] {
        let response: TurnSteerResponse = app
            .request(|request_id| ClientRequest::TurnSteer {
                request_id,
                params: TurnSteerParams {
                    thread_id: thread.id.clone(),
                    expected_turn_id: turn.id.clone(),
                    client_user_message_id: Some(client_id.into()),
                    input: vec![UserInput::Text {
                        text: text.into(),
                        text_elements: Vec::new(),
                    }],
                    responsesapi_client_metadata: None,
                    additional_context: None,
                },
            })
            .await?;
        assert_eq!(
            response,
            TurnSteerResponse {
                turn_id: turn.id.clone()
            }
        );
    }
    for (expected_turn_id, client_id, cancelled) in [
        ("different-turn", "recall", false),
        (turn.id.as_str(), "unknown", false),
        (turn.id.as_str(), "duplicate", false),
        (turn.id.as_str(), "recall", true),
        (turn.id.as_str(), "recall", false),
    ] {
        let response: TurnSteerCancelResponse = app
            .request(|request_id| ClientRequest::TurnSteerCancel {
                request_id,
                params: TurnSteerCancelParams {
                    thread_id: thread.id.clone(),
                    expected_turn_id: expected_turn_id.into(),
                    client_user_message_id: client_id.into(),
                },
            })
            .await?;
        assert_eq!(response, TurnSteerCancelResponse { cancelled });
    }
    first_tx.send(()).unwrap();
    timeout(
        Duration::from_secs(30),
        server.wait_for_request_count(/*count*/ 2),
    )
    .await?;
    let requests = server.requests().await;
    let body: Value = serde_json::from_slice(&requests[1])?;
    let texts: Vec<&str> = body["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["role"] == "user")
        .filter_map(|item| item["content"].as_array())
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .filter(|text| {
            [
                "initial message",
                "recalled message",
                "retained message",
                "duplicate one",
                "duplicate two",
            ]
            .contains(text)
        })
        .collect();
    assert_eq!(
        texts,
        vec![
            "initial message",
            "retained message",
            "duplicate one",
            "duplicate two"
        ]
    );
    let response: TurnSteerCancelResponse = app
        .request(|request_id| ClientRequest::TurnSteerCancel {
            request_id,
            params: TurnSteerCancelParams {
                thread_id: thread.id.clone(),
                expected_turn_id: turn.id.clone(),
                client_user_message_id: "keep".into(),
            },
        })
        .await?;
    assert_eq!(response, TurnSteerCancelResponse { cancelled: false });
    second_tx.send(()).unwrap();
    let notification = timeout(
        Duration::from_secs(30),
        app.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    let completed: TurnCompletedNotification =
        serde_json::from_value(notification.params.unwrap())?;
    assert_eq!(
        (
            completed.turn.id,
            completed.turn.status,
            completed.turn.error
        ),
        (turn.id, TurnStatus::Completed, None)
    );
    server.shutdown().await;
    Ok(())
}
