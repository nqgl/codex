//! Resolves collaboration catalog text and bundled presets while retaining their source.
//! Runtime consumers use catalog overrides first, then the selected mode's settings.

use super::ResolvedMessage;
use codex_collaboration_mode_templates::DEFAULT;
use codex_collaboration_mode_templates::PLAN;
use codex_protocol::openai_models::CollaborationModeMessages;

#[derive(Debug, Clone, Copy)]
pub struct ResolvedCollaborationModeMessages<'a> {
    pub prompt_mode: codex_protocol::config_types::PromptMode,
    pub default: ResolvedMessage<'a>,
    pub plan: ResolvedMessage<'a>,
}

impl<'a> ResolvedCollaborationModeMessages<'a> {
    pub(crate) fn new_for_mode(
        messages: Option<&'a CollaborationModeMessages>,
        mode: codex_protocol::config_types::PromptMode,
    ) -> Self {
        Self {
            prompt_mode: mode,
            default: ResolvedMessage::new(
                messages.and_then(|m| m.default.as_deref()),
                mode.select(DEFAULT, include_str!("../../templates/upstream/default.md")),
            ),
            plan: ResolvedMessage::new(
                messages.and_then(|m| m.plan.as_deref()),
                mode.select(PLAN, include_str!("../../templates/upstream/plan.md")),
            ),
        }
    }
}
