use super::ContextualUserFragment;
use codex_protocol::models::ContentItemKind;

/// Short, thread-bound instructions for a user-registered group member.
pub struct GroupMailInstructions {
    name: String,
    group: String,
}

impl GroupMailInstructions {
    pub fn new(name: String, group: String) -> Self {
        Self { name, group }
    }
}

impl ContextualUserFragment for GroupMailInstructions {
    fn role(&self) -> &'static str {
        "developer"
    }

    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("group_mail.instructions".to_string())
    }

    fn requires_separate_message(&self) -> bool {
        true
    }

    fn markers(&self) -> (&'static str, &'static str) {
        ("", "")
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("", "")
    }

    fn body(&self) -> String {
        format!(
            "You are {} in {}. Use send_to or broadcast with high or low priority to message peers in your group.",
            self.name, self.group
        )
    }
}
