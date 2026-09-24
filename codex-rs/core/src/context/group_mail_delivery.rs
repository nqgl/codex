use super::ContextualUserFragment;
use codex_protocol::models::ContentItemKind;

/// One bounded, ordered batch of complete peer messages.
pub struct GroupMailDelivery {
    recipient: String,
    messages: Vec<(String, String)>,
}

impl GroupMailDelivery {
    pub fn new(recipient: String, messages: Vec<(String, String)>) -> Self {
        Self {
            recipient,
            messages,
        }
    }
}

impl ContextualUserFragment for GroupMailDelivery {
    fn role(&self) -> &'static str {
        "assistant"
    }

    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("group_mail.delivery".to_string())
    }

    fn markers(&self) -> (&'static str, &'static str) {
        ("", "")
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("", "")
    }

    fn body(&self) -> String {
        let recipient = &self.recipient;
        self.messages
            .iter()
            .map(|(sender, body)| format!("From {sender} to {recipient}:\n{body}"))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}
