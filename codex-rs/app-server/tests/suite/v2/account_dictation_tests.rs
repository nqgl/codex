use anyhow::Result;
use app_test_support::ChatGptIdTokenClaims;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use app_test_support::encode_id_token;
use codex_app_server_protocol::LoginAccountResponse;
use codex_app_server_protocol::ThreadRealtimeAppendAudioParams;
use codex_app_server_protocol::ThreadRealtimeAppendAudioResponse;
use codex_app_server_protocol::ThreadRealtimeAudioChunk;
use codex_app_server_protocol::ThreadRealtimeStartParams;
use codex_app_server_protocol::ThreadRealtimeStartResponse;
use codex_app_server_protocol::ThreadStartParams;
use core_test_support::responses::WebSocketConnectionConfig;
use core_test_support::responses::start_websocket_server_with_headers;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;
use test_case::test_case;
use tokio::time::Duration;
use tokio::time::timeout;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[test_case(200; "streaming_work_account")]
#[test_case(403; "denied_without_api_fallback")]
#[tokio::test]
async fn account_dictation_uses_selected_workspace_and_never_falls_back_to_personal_api_key(
    status: u16,
) -> Result<()> {
    let home = TempDir::new()?;
    let backend = MockServer::start().await;
    let forbidden_api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/codex/config/bundle"))
        .respond_with(ResponseTemplate::new(/*s*/ 200).set_body_json(json!({})))
        .mount(&backend)
        .await;
    let stream = start_websocket_server_with_headers(vec![WebSocketConnectionConfig {
        requests: vec![
            vec![json!({"type":"session.started","session":{"config":{"provider_mode":"streaming_sse","transcript_delivery_mode":"segment"}}})],
            vec![json!({"type":"transcript.segment","utterance_id":"one","revision":1,"text":"live words"})],
            vec![json!({"type":"transcript.final","utterance_id":"one","revision":2,"text":"Live words."}), json!({"type":"session.updated","session":{"status":"closed"}})],
        ],
        response_headers: vec![("sec-websocket-protocol".into(), "dictation".into())],
        accept_delay: None,
        close_after_requests: true,
    }]).await;
    MockResponsesConfig::new(&backend.uri())
        .with_provider_name("OpenAI")
        .with_provider_config("requires_openai_auth = true")
        .with_root_config(&format!(
            "chatgpt_base_url = {:?}\nexperimental_realtime_ws_base_url = {:?}",
            backend.uri(),
            forbidden_api.uri()
        ))
        .write(home.path())?;
    let token = encode_id_token(
        &ChatGptIdTokenClaims::new()
            .plan_type("business")
            .chatgpt_account_id("work-workspace"),
    )?;
    let response = if status == 200 {
        ResponseTemplate::new(status).set_body_json(
            json!({"websocketUrl":stream.uri(),"protocols":["dictation","ephemeral-ticket"]}),
        )
    } else {
        ResponseTemplate::new(status).set_body_string("private-upstream-error")
    };
    Mock::given(method("POST"))
        .and(path("/codex/dictation-stream-connect-info"))
        .and(header("authorization", format!("Bearer {token}")))
        .and(header("chatgpt-account-id", "work-workspace"))
        .respond_with(response)
        .expect(/*r*/ 1)
        .mount(&backend)
        .await;
    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .without_managed_config()
        .with_env_overrides(&[("OPENAI_API_KEY", Some("personal-key-must-not-be-used"))])
        .build_initialized()
        .await?;
    let login_id = app
        .send_chatgpt_auth_tokens_login_request(
            token,
            "work-workspace".into(),
            Some("business".into()),
        )
        .await?;
    let _: LoginAccountResponse = app.read_response(login_id).await?;
    let thread = app.start_thread(ThreadStartParams::default()).await?.thread;
    let params: ThreadRealtimeStartParams = serde_json::from_value(json!({
        "threadId":thread.id,"sessionType":"transcription","version":"v2","outputModality":"text",
        "transport":{"type":"websocket"},"includeStartupContext":false,"clientManagedHandoffs":true
    }))?;
    let request = app.send_thread_realtime_start_request(params).await?;
    let _: ThreadRealtimeStartResponse = app.read_response(request).await?;
    if status == 200 {
        timeout(
            Duration::from_secs(/*secs*/ 15),
            app.read_stream_until_notification_message("thread/realtime/started"),
        )
        .await??;
        let request = app
            .send_thread_realtime_append_audio_request(ThreadRealtimeAppendAudioParams {
                thread_id: thread.id.clone(),
                audio: ThreadRealtimeAudioChunk {
                    data: "AAA=".into(),
                    sample_rate: 24000,
                    num_channels: 1,
                    ..Default::default()
                },
                commit: false,
            })
            .await?;
        let _: ThreadRealtimeAppendAudioResponse = app.read_response(request).await?;
        let delta = timeout(
            Duration::from_secs(/*secs*/ 15),
            app.read_stream_until_notification_message("thread/realtime/transcript/delta"),
        )
        .await??;
        assert_eq!(
            delta.params.expect("transcript delta params")["delta"],
            "live words"
        );
        let request = app
            .send_thread_realtime_append_audio_request(ThreadRealtimeAppendAudioParams {
                thread_id: thread.id.clone(),
                audio: ThreadRealtimeAudioChunk {
                    sample_rate: 24000,
                    num_channels: 1,
                    ..Default::default()
                },
                commit: true,
            })
            .await?;
        let _: ThreadRealtimeAppendAudioResponse = app.read_response(request).await?;
        let done = timeout(
            Duration::from_secs(/*secs*/ 15),
            app.read_stream_until_notification_message("thread/realtime/transcript/done"),
        )
        .await??;
        assert_eq!(
            done.params.expect("final transcript params")["text"],
            "Live words."
        );
    } else {
        let notification = timeout(
            Duration::from_secs(/*secs*/ 15),
            app.read_stream_until_notification_message("thread/realtime/error"),
        )
        .await??;
        let error = notification
            .params
            .expect("dictation error params")
            .to_string();
        assert!(error.contains("HTTP 403"), "{error}");
        assert!(!error.contains("private-upstream-error"));
        assert!(!error.contains("personal-key-must-not-be-used"));
        assert!(stream.connections().is_empty());
    }
    assert!(
        forbidden_api
            .received_requests()
            .await
            .expect("recorded API requests")
            .is_empty()
    );
    backend.verify().await;
    stream.shutdown().await;
    Ok(())
}
