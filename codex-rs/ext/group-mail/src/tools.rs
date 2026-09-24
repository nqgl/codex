use crate::extension::Binding;
use crate::store::Priority;
use codex_extension_api::FunctionCallError;
use codex_extension_api::JsonToolOutput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolExecutor;
use codex_extension_api::ToolExecutorFuture;
use codex_extension_api::ToolName;
use codex_extension_api::ToolOutput;
use codex_extension_api::ToolSpec;
use codex_tools::ResponsesApiTool;
use codex_tools::parse_tool_input_schema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub(super) struct GroupMailTool {
    name: &'static str,
    binding: Arc<Binding>,
}

impl GroupMailTool {
    pub(super) fn new(name: &'static str, binding: Arc<Binding>) -> Self {
        Self { name, binding }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendArgs {
    recipients: Option<Vec<String>>,
    message: String,
    priority: Priority,
}

impl<'call> ToolExecutor<ToolCall<'call>> for GroupMailTool {
    fn tool_name(&self) -> ToolName {
        ToolName::new(/*namespace*/ None, self.name)
    }

    fn spec(&self) -> ToolSpec {
        let recipients = json!({"type":"array","items":{"type":"string"},"minItems":1,"maxItems":32,"description":"Names of peers in your group."});
        let mut properties = serde_json::Map::new();
        if self.name == "send_to" {
            properties.insert("recipients".into(), recipients);
        }
        properties.insert("message".into(), json!({"type":"string","description":"Message text, at most 4,000 UTF-8 bytes. Never truncated."}));
        properties.insert("priority".into(), json!({"type":"string","enum":["high","low"],"description":"High arrives at the next safe continuation; low waits until the peer is idle."}));
        let required = if self.name == "send_to" {
            vec!["recipients", "message", "priority"]
        } else {
            vec!["message", "priority"]
        };
        let parameters = json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
        ToolSpec::Function(ResponsesApiTool {
            name: self.name.into(),
            description: if self.name == "send_to" {
                "Send one message to named peers in your group. Reports which recipients are offline."
            } else {
                "Send one message to every other peer in your group. Reports which peers are offline."
            }.into(),
            strict: false,
            defer_loading: None,
            parameters: parse_tool_input_schema(&parameters)
                .unwrap_or_else(|error| panic!("group mail schema must parse: {error}")),
            output_schema: None,
        })
    }

    fn handle<'a>(&'a self, call: ToolCall<'call>) -> ToolExecutorFuture<'a>
    where
        'call: 'a,
    {
        Box::pin(async move {
            let raw = call.function_arguments()?;
            if raw.len() > 12_000 {
                return Err(FunctionCallError::RespondToModel(
                    "Group-mail arguments exceed 12,000 bytes; nothing was sent.".into(),
                ));
            }
            let args: SendArgs = serde_json::from_str(raw)
                .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
            if self.name == "send_to" && args.recipients.is_none() {
                return Err(FunctionCallError::RespondToModel(
                    "send_to requires recipients; nothing was sent.".into(),
                ));
            }
            if self.name == "broadcast" && args.recipients.is_some() {
                return Err(FunctionCallError::RespondToModel(
                    "broadcast takes no recipients; nothing was sent.".into(),
                ));
            }
            let receipt = self
                .binding
                .store
                .send(
                    self.binding.thread_id,
                    args.recipients.as_deref(),
                    &args.message,
                    args.priority,
                )
                .await
                .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
            let online = receipt
                .recipients
                .iter()
                .filter(|peer| peer.online)
                .map(|peer| peer.name.as_str())
                .collect::<Vec<_>>();
            let offline = receipt
                .recipients
                .iter()
                .filter(|peer| !peer.online)
                .map(|peer| peer.name.as_str())
                .collect::<Vec<_>>();
            let value = json!({"sent":true,"online":online,"offline":offline});
            Ok(Box::new(JsonToolOutput::new(value)) as Box<dyn ToolOutput>)
        })
    }
}
