use super::*;
use crate::AuthProvider;
use codex_http_client::OutboundProxyPolicy;
use codex_protocol::protocol::RealtimeAudioFrame;
use http::HeaderMap;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tokio::net::TcpListener;
use tungstenite::handshake::server::Request;
use tungstenite::handshake::server::Response;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

struct WorkAuth;
impl AuthProvider for WorkAuth {
    fn add_auth_headers(&self, headers: &mut HeaderMap) {
        headers.insert("authorization", "Bearer work-token".parse().unwrap());
        headers.insert("chatgpt-account-id", "work-account".parse().unwrap());
    }
}

struct UnscopedAuth;
impl AuthProvider for UnscopedAuth {
    fn add_auth_headers(&self, headers: &mut HeaderMap) {
        headers.insert("authorization", "Bearer work-token".parse().unwrap());
        headers.insert("chatgpt-account-id", "".parse().unwrap());
    }
}

#[tokio::test]
async fn refuses_unscoped_account_before_requesting_a_connection() {
    let server = MockServer::start().await;
    let factory = HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault);
    let error = ChatgptDictationStream::connect(&server.uri(), Arc::new(UnscopedAuth), &factory)
        .await
        .err()
        .unwrap()
        .to_string();
    assert_eq!(
        error,
        "stream error: ChatGPT dictation: a signed-in ChatGPT workspace is required"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn account_stream_emits_partial_before_commit_and_authoritative_revision_after_close() {
    let http = MockServer::start().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    Mock::given(method("POST")).and(path("/codex/dictation-stream-connect-info"))
        .and(header("authorization", "Bearer work-token"))
        .and(header("chatgpt-account-id", "work-account"))
        .respond_with(ResponseTemplate::new(/*s*/ 200).set_body_json(json!({
            "websocketUrl":format!("ws://{address}/stream"),"protocols":["dictation", "ephemeral-ticket"]
        }))).expect(/*r*/ 1).mount(&http).await;
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_hdr_async(
            tcp,
            |request: &Request, mut response: Response| {
                assert!(request.headers().get("authorization").is_none());
                assert!(request.headers().get("chatgpt-account-id").is_none());
                assert_eq!(
                    request.headers()[SEC_WEBSOCKET_PROTOCOL],
                    "dictation, ephemeral-ticket"
                );
                response
                    .headers_mut()
                    .insert(SEC_WEBSOCKET_PROTOCOL, "dictation".parse().unwrap());
                Ok(response)
            },
        )
        .await
        .unwrap();
        let start: Value =
            serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(
            start,
            json!({"type":"session.start","config":{
                "input_audio_format":"pcm16","sample_rate_hz":24000,"num_channels":1,
                "max_buffer_size_bytes":4194304,"max_utterance_duration_ms":30000,"session_ttl_ms":300000,
                "provider_mode":"streaming_sse","transcript_delivery_mode":"segment",
                "vad":{"type":"server_vad","threshold":0.5,"prefix_padding_ms":300,"silence_duration_ms":500}
            }})
        );
        ws.send(Message::text(
            json!({"type":"session.started","session":{"config":{
                "provider_mode":"streaming_sse","transcript_delivery_mode":"segment"
            }}})
            .to_string(),
        ))
        .await
        .unwrap();
        let audio: Value =
            serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(audio, json!({"type":"audio.append","audio":"AAA="}));
        ws.send(Message::text(json!({"type":"transcript.segment","utterance_id":"one","revision":1,"text":"hello wrld"}).to_string())).await.unwrap();
        let close: Value =
            serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(close, json!({"type":"session.close"}));
        for event in [
            json!({"type":"transcript.final","utterance_id":"one","revision":2,"text":"Hello world!"}),
            json!({"type":"transcript.segment","utterance_id":"one","revision":1,"text":"stale"}),
            json!({"type":"session.updated","session":{"status":"closed"}}),
        ] {
            ws.send(Message::text(event.to_string())).await.unwrap();
        }
        let _ = ws.next().await;
    });
    let factory = HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault);
    let stream = ChatgptDictationStream::connect(&http.uri(), Arc::new(WorkAuth), &factory)
        .await
        .unwrap();
    let (audio_tx, audio_rx) = async_channel::bounded(/*cap*/ 4);
    let (event_tx, event_rx) = async_channel::bounded(/*cap*/ 4);
    let client = tokio::spawn(stream.run(audio_rx, event_tx));
    audio_tx
        .send(ConversationAudioParams {
            frame: RealtimeAudioFrame {
                data: "AAA=".into(),
                sample_rate: 24000,
                num_channels: 1,
                samples_per_channel: None,
                item_id: None,
            },
            commit: false,
        })
        .await
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(/*secs*/ 5), event_rx.recv())
            .await
            .unwrap()
            .unwrap(),
        RealtimeEvent::InputTranscriptDelta(RealtimeTranscriptDelta {
            delta: "hello wrld".into()
        })
    );
    audio_tx
        .send(ConversationAudioParams {
            frame: RealtimeAudioFrame {
                data: String::new(),
                sample_rate: 24000,
                num_channels: 1,
                samples_per_channel: None,
                item_id: None,
            },
            commit: true,
        })
        .await
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(/*secs*/ 5), event_rx.recv())
            .await
            .unwrap()
            .unwrap(),
        RealtimeEvent::InputTranscriptDone(RealtimeTranscriptDone {
            text: "Hello world!".into()
        })
    );
    client.await.unwrap().unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn bootstrap_denials_and_redirects_fail_without_exposing_bodies_or_tickets() {
    let factory = HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault);
    for status in [302, 401, 403] {
        let server = MockServer::start().await;
        Mock::given(path("/codex/dictation-stream-connect-info"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("location", format!("{}/leak", server.uri()))
                    .set_body_string("sensitive-ticket-and-transcript"),
            )
            .expect(/*r*/ 1)
            .mount(&server)
            .await;
        let error = ChatgptDictationStream::connect(&server.uri(), Arc::new(WorkAuth), &factory)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(&status.to_string()));
        assert!(!error.contains("sensitive-ticket-and-transcript"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn rejects_untrusted_bootstrap_and_stream_destinations_before_sending_credentials() {
    let factory = HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault);
    assert!(
        ChatgptDictationStream::connect(
            "https://chatgpt.com.evil.example",
            Arc::new(WorkAuth),
            &factory
        )
        .await
        .is_err()
    );
    let server = MockServer::start().await;
    Mock::given(path("/codex/dictation-stream-connect-info"))
        .respond_with(ResponseTemplate::new(/*s*/ 200).set_body_json(json!({"websocketUrl":"wss://evil.example/secret-ticket","protocols":["secret-ticket"]})))
        .mount(&server).await;
    let error = ChatgptDictationStream::connect(&server.uri(), Arc::new(WorkAuth), &factory)
        .await
        .err()
        .unwrap()
        .to_string();
    assert_eq!(
        error,
        "stream error: ChatGPT dictation: untrusted streaming destination or invalid connection ticket"
    );
}

#[test]
fn transcript_requires_final_results_and_caps_session_memory() {
    let mut transcript = Transcript::default();
    transcript.update(&json!({"type":"transcript.segment","utterance_id":"one","revision":1,"text":"partial"})).unwrap();
    assert!(transcript.finish().is_err());
    let mut transcript = Transcript::default();
    assert!(transcript.update(&json!({"type":"transcript.final","utterance_id":"one","revision":1,"text":"x".repeat(MAX_TEXT + 1)})).is_err());
}
