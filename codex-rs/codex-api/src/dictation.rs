//! Account-backed streaming dictation, matching the desktop app's session protocol.
//! Audio, connection tickets, transcript text, and remote error bodies are never logged here.

use crate::ApiError;
use crate::SharedAuthProvider;
use async_channel::Receiver;
use async_channel::Sender;
use base64::Engine;
use codex_http_client::ClientRouteClass;
use codex_http_client::HttpClientFactory;
use codex_http_client::RouteAwareClientPool;
use codex_http_client::is_allowed_chatgpt_host;
use codex_protocol::protocol::ConversationAudioParams;
use codex_protocol::protocol::RealtimeEvent;
use codex_protocol::protocol::RealtimeTranscriptDelta;
use codex_protocol::protocol::RealtimeTranscriptDone;
use codex_websocket_client::WebSocketConnection;
use codex_websocket_client::WebSocketConnector;
use futures::SinkExt;
use futures::StreamExt;
use http::header::SEC_WEBSOCKET_PROTOCOL;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;
use tokio::time::Instant;
use tokio::time::timeout;
use tokio::time::timeout_at;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::WebSocketConfig;
use url::Url;

const MAX_TEXT: usize = 32 * 1024;
const MAX_MESSAGE: usize = 128 * 1024;

// Deliberately not Debug: both fields can contain short-lived credentials.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectInfo {
    websocket_url: String,
    protocols: Vec<String>,
}

/// A workspace-authenticated dictation session. There is no API-key or batch fallback.
pub struct ChatgptDictationStream {
    socket: WebSocketConnection,
}

fn failure(message: &str) -> ApiError {
    ApiError::Stream(format!("ChatGPT dictation: {message}"))
}

impl ChatgptDictationStream {
    pub async fn connect(
        base_url: &str,
        auth: SharedAuthProvider,
        factory: &HttpClientFactory,
        routing_headers: http::HeaderMap,
    ) -> Result<Self, ApiError> {
        timeout(
            Duration::from_secs(/*secs*/ 10),
            Self::connect_inner(base_url, auth, factory, routing_headers),
        )
        .await
        .map_err(|_| failure("connection timed out"))?
    }

    async fn connect_inner(
        base_url: &str,
        auth: SharedAuthProvider,
        factory: &HttpClientFactory,
        routing_headers: http::HeaderMap,
    ) -> Result<Self, ApiError> {
        let mut base = Url::parse(base_url).map_err(|_| failure("invalid account service URL"))?;
        let local = base.host_str().is_some_and(|host| {
            host.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || !(base.scheme() == "https" && base.host_str().is_some_and(is_allowed_chatgpt_host)
                || local && matches!(base.scheme(), "http" | "https"))
        {
            return Err(failure(
                "account service must be a trusted ChatGPT HTTPS origin",
            ));
        }
        if base.path() == "/" && !local {
            base.set_path("/backend-api");
        }
        let url = format!(
            "{}/codex/dictation-stream-connect-info",
            base.as_str().trim_end_matches('/')
        );
        let headers = auth
            .resolve_auth_headers()
            .await
            .map_err(|_| failure("could not obtain account credentials"))?;
        if headers
            .get("ChatGPT-Account-Id")
            .and_then(|value| value.to_str().ok())
            .is_none_or(|id| id.trim().is_empty())
            || !headers.contains_key(http::header::AUTHORIZATION)
        {
            return Err(failure("a signed-in ChatGPT workspace is required"));
        }
        let http = RouteAwareClientPool::with_chatgpt_cloudflare_cookies_without_redirects_or_request_logging(factory.clone(), ClientRouteClass::Api);
        let mut response = http
            .request(http::Method::POST, &url)
            .headers(routing_headers)
            .headers(headers)
            .send()
            .await
            .map_err(|_| failure("could not request a streaming connection"))?;
        if !response.status().is_success() {
            return Err(failure(&format!(
                "connection request returned HTTP {}; no API-key fallback was attempted",
                response.status().as_u16()
            )));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| failure("invalid connection response"))?
        {
            if body.len() + chunk.len() > 16 * 1024 {
                return Err(failure("connection response too large"));
            }
            body.extend_from_slice(&chunk);
        }
        let info: ConnectInfo =
            serde_json::from_slice(&body).map_err(|_| failure("invalid connection response"))?;
        let ws_url = Url::parse(&info.websocket_url)
            .map_err(|_| failure("invalid streaming destination"))?;
        let ws_local = local && ws_url.host_str() == base.host_str();
        let trusted_host = ws_url.host_str().is_some_and(|host| {
            is_allowed_chatgpt_host(host) || host == "openai.com" || host.ends_with(".openai.com")
        });
        if !ws_url.username().is_empty()
            || ws_url.password().is_some()
            || ws_url.fragment().is_some()
            || !(ws_url.scheme() == "wss" && trusted_host || ws_url.scheme() == "ws" && ws_local)
            || info.protocols.is_empty()
            || info.protocols.len() > 8
            || info.protocols.iter().any(|p| {
                p.is_empty()
                    || p.len() > 8192
                    || p.bytes()
                        .any(|b| !b.is_ascii_alphanumeric() && !b"!#$%&'*+-.^_`|~".contains(&b))
            })
        {
            return Err(failure(
                "untrusted streaming destination or invalid connection ticket",
            ));
        }
        let mut request = info
            .websocket_url
            .into_client_request()
            .map_err(|_| failure("invalid streaming request"))?;
        let mut protocols = http::HeaderValue::from_str(&info.protocols.join(", "))
            .map_err(|_| failure("invalid connection ticket"))?;
        protocols.set_sensitive(/*val*/ true);
        request
            .headers_mut()
            .insert(SEC_WEBSOCKET_PROTOCOL, protocols);
        // Only short-lived subprotocol tickets go to the WebSocket, never the account bearer.
        let connector =
            WebSocketConnector::new(factory).map_err(|_| failure("TLS setup failed"))?;
        let (socket, _) = connector
            .connect(
                request,
                WebSocketConfig::default()
                    .max_message_size(Some(MAX_MESSAGE))
                    .max_frame_size(Some(MAX_MESSAGE)),
            )
            .await
            .map_err(|_| failure("streaming connection failed"))?;
        let mut stream = Self { socket };
        stream.send(json!({"type":"session.start","config":{
            "input_audio_format":"pcm16","sample_rate_hz":24000,"num_channels":1,
            "max_buffer_size_bytes":4194304,"max_utterance_duration_ms":30000,"session_ttl_ms":300000,
            "provider_mode":"streaming_sse","transcript_delivery_mode":"segment",
            "vad":{"type":"server_vad","threshold":0.5,"prefix_padding_ms":300,"silence_duration_ms":500}
        }})).await?;
        let event = stream.receive().await?;
        if event["type"] != "session.started"
            || event["session"]["config"]["provider_mode"] != "streaming_sse"
            || event["session"]["config"]["transcript_delivery_mode"] != "segment"
        {
            return Err(failure(
                "server did not start the requested streaming session",
            ));
        }
        Ok(stream)
    }

    async fn send(&mut self, value: Value) -> Result<(), ApiError> {
        timeout(
            Duration::from_secs(/*secs*/ 8),
            self.socket.send(Message::Text(value.to_string().into())),
        )
        .await
        .map_err(|_| failure("streaming send timed out"))?
        .map_err(|_| failure("streaming send failed"))
    }

    async fn receive(&mut self) -> Result<Value, ApiError> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    return serde_json::from_str(&text)
                        .map_err(|_| failure("invalid streaming event"));
                }
                Some(Ok(Message::Ping(payload))) => self
                    .socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|_| failure("streaming heartbeat failed"))?,
                Some(Ok(Message::Pong(_))) => {}
                _ => return Err(failure("stream closed before completion")),
            }
        }
    }

    /// Forward bounded PCM audio and partial transcripts until the server confirms finalization.
    pub async fn run(
        mut self,
        audio: Receiver<ConversationAudioParams>,
        events: Sender<RealtimeEvent>,
    ) -> Result<(), ApiError> {
        let mut transcript = Transcript::default();
        let mut closing = false;
        let mut deadline = Instant::now() + Duration::from_secs(/*secs*/ 300);
        loop {
            let event = tokio::select! {
                frame = audio.recv(), if !closing => {
                    let frame = frame.map_err(|_| failure("audio input closed"))?;
                    if frame.frame.sample_rate != 24000 || frame.frame.num_channels != 1 || frame.frame.data.len() > MAX_MESSAGE {
                        return Err(failure("expected bounded 24 kHz mono PCM audio"));
                    }
                    if base64::engine::general_purpose::STANDARD.decode(&frame.frame.data).map_err(|_| failure("invalid PCM encoding"))?.len() % 2 != 0 {
                        return Err(failure("incomplete PCM sample"));
                    }
                    if !frame.frame.data.is_empty() { self.send(json!({"type":"audio.append","audio":frame.frame.data})).await?; }
                    if frame.commit {
                        self.send(json!({"type":"session.close"})).await?;
                        closing = true;
                        deadline = Instant::now() + Duration::from_secs(/*secs*/ 8);
                    }
                    continue;
                },
                event = timeout_at(deadline, self.receive()) => event.map_err(|_| failure("session timed out"))??,
            };
            match event["type"].as_str() {
                Some("transcript.segment" | "transcript.final") => {
                    if let Some(delta) = transcript.update(&event)? {
                        events
                            .send(RealtimeEvent::InputTranscriptDelta(
                                RealtimeTranscriptDelta { delta },
                            ))
                            .await
                            .map_err(|_| failure("transcript receiver closed"))?;
                    }
                }
                Some("session.updated") if event["session"]["status"] == "closed" && closing => {
                    let text = transcript.finish()?;
                    events
                        .send(RealtimeEvent::InputTranscriptDone(RealtimeTranscriptDone {
                            text,
                        }))
                        .await
                        .map_err(|_| failure("transcript receiver closed"))?;
                    let _ = timeout(Duration::from_secs(/*secs*/ 1), self.socket.close()).await;
                    return Ok(());
                }
                Some("transcript.failed" | "session.error") => {
                    return Err(failure("service reported a transcription error"));
                }
                Some(
                    "session.updated" | "speech.started" | "speech.stopped" | "transcript.delta"
                    | "asset.ready" | "asset.committed" | "asset.failed",
                ) => {}
                _ => return Err(failure("unexpected streaming event")),
            }
        }
    }
}

#[cfg(test)]
#[path = "dictation_tests.rs"]
mod tests;

#[derive(Default)]
struct Transcript {
    utterances: Vec<Utterance>,
    preview: String,
}

struct Utterance {
    id: String,
    revision: u64,
    text: String,
    finalized: bool,
}

impl Transcript {
    fn update(&mut self, event: &Value) -> Result<Option<String>, ApiError> {
        let id = event["utterance_id"]
            .as_str()
            .filter(|id| id.len() <= 256)
            .ok_or_else(|| failure("invalid utterance ID"))?;
        let revision = event["revision"]
            .as_u64()
            .ok_or_else(|| failure("invalid transcript revision"))?;
        let text = event["text"]
            .as_str()
            .filter(|text| text.len() <= MAX_TEXT)
            .ok_or_else(|| failure("transcript exceeds limit"))?;
        let final_text = event["type"] == "transcript.final";
        if let Some(utterance) = self.utterances.iter_mut().find(|u| u.id == id) {
            if revision < utterance.revision || utterance.finalized && !final_text {
                return Ok(None);
            }
            *utterance = Utterance {
                id: id.to_string(),
                revision,
                text: text.to_string(),
                finalized: final_text,
            };
        } else {
            if self.utterances.len() >= 512 {
                return Err(failure("too many utterances"));
            }
            self.utterances.push(Utterance {
                id: id.to_string(),
                revision,
                text: text.to_string(),
                finalized: final_text,
            });
        }
        if self
            .utterances
            .iter()
            .map(|u| u.text.len() + 1)
            .sum::<usize>()
            > MAX_TEXT
        {
            return Err(failure("transcript exceeds limit"));
        }
        let joined = self
            .utterances
            .iter()
            .map(|u| u.text.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        // Existing delta consumers are append-only. Revisions are corrected by the authoritative
        // final result rather than appending contradictory text or treating partials as final.
        let delta = joined
            .strip_prefix(&self.preview)
            .filter(|suffix| !suffix.is_empty())
            .map(str::to_owned);
        if delta.is_some() {
            self.preview = joined;
        }
        Ok(delta)
    }

    fn finish(self) -> Result<String, ApiError> {
        if self.utterances.iter().any(|u| !u.finalized) {
            return Err(failure("session ended with an unfinished transcript"));
        }
        Ok(self
            .utterances
            .into_iter()
            .map(|u| u.text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" "))
    }
}
