use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::UserInput;
use core_test_support::responses;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

#[test_case::test_case("custom"; "custom_daemon")]
#[test_case::test_case("upstream"; "upstream_daemon")]
#[tokio::test]
async fn clients_choose_prompt_mode_independently_of_daemon_startup(
    default_mode: &str,
) -> Result<()> {
    let server = responses::start_mock_server().await;
    let home = TempDir::new()?;
    MockResponsesConfig::new(&server.uri())
        .with_root_config("instructions = 'Shared older-model override.'")
        .write(home.path())?;
    let daemon_mode = format!("prompt_mode=\"{default_mode}\"");
    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .with_args(&["-c", &daemon_mode])
        .build_initialized()
        .await?;
    for mode in ["upstream", "custom", "upstream"] {
        let mock = responses::mount_sse_once(
            &server,
            responses::sse(vec![
                responses::ev_response_created(mode),
                responses::ev_assistant_message("answer", "Done."),
                responses::ev_completed(mode),
            ]),
        )
        .await;
        let id = app
            .send_thread_start_request_with_auto_env(ThreadStartParams {
                config: Some(HashMap::from([("prompt_mode".to_owned(), json!(mode))])),
                developer_instructions: Some("Terminal visualization feature guidance.".to_owned()),
                allow_provider_model_fallback: true,
                ..Default::default()
            })
            .await?;
        let started: ThreadStartResponse = app.read_response(id).await?;
        app.send_turn_start_request(TurnStartParams {
            thread_id: started.thread.id,
            input: vec![UserInput::Text {
                text: "Check this session's prompt.".to_owned(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
        let _: TurnCompletedNotification = app.read_notification("turn/completed").await?;
        let request = mock.single_request();
        assert_eq!(
            request.body_contains_text("Shared older-model override."),
            mode == "custom"
        );
        assert!(request.body_contains_text("Terminal visualization feature guidance."));
        assert!(
            !request.body_json()["instructions"]
                .as_str()
                .expect("base instructions")
                .is_empty()
        );
    }
    Ok(())
}
