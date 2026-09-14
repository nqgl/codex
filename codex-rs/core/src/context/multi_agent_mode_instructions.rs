use super::ContextualUserFragment;
use codex_protocol::config_types::MultiAgentMode;
use codex_protocol::models::ContentItemKind;
use codex_protocol::protocol::MULTI_AGENT_MODE_CLOSE_TAG;
use codex_protocol::protocol::MULTI_AGENT_MODE_OPEN_TAG;

const EXPLICIT_REQUEST_ONLY_MULTI_AGENT_MODE_TEXT: &str = "Any earlier instruction enabling proactive multi-agent delegation no longer applies. Do not spawn sub-agents unless the user or applicable AGENTS.md or skill instructions explicitly ask for sub-agents, delegation, or parallel agent work.";
const BALANCED_MULTI_AGENT_MODE_TEXT: &str = "Balanced multi-agent delegation is active. You may spawn a sub-agent without an explicit user request when a concrete, bounded task is genuinely independent and can run alongside useful local work, or when one focused independent review would materially improve confidence. Do not delegate routine steps, tightly coupled work, or work that would leave the primary thread merely waiting. Prefer one narrowly scoped agent; add more only for genuinely independent high-risk questions. User requests override this hint. This mode remains active until a later multi-agent mode developer message changes it.";
const PROACTIVE_MULTI_AGENT_MODE_TEXT: &str = "Proactive multi-agent delegation is active. Any earlier developer instruction requiring an explicit user request before spawning sub-agents no longer applies. User requests override this hint. Actively consider sub-agents for separable work, and use them when doing so would materially improve speed or quality. There's no need to fill slots by default; keep load-bearing synthesis and overlapping edits in the primary thread. This mode remains active until a later multi-agent mode developer message changes it.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MultiAgentModeInstructions {
    multi_agent_mode: MultiAgentMode,
}

impl MultiAgentModeInstructions {
    pub(super) fn from_mode(multi_agent_mode: MultiAgentMode) -> Option<Self> {
        if matches!(
            &multi_agent_mode,
            MultiAgentMode::Custom(hint_text) if hint_text.is_empty()
        ) {
            return None;
        }

        Some(Self { multi_agent_mode })
    }
}

impl ContextualUserFragment for MultiAgentModeInstructions {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("multi_agent.mode_instructions".to_string())
    }

    fn role(&self) -> &'static str {
        "developer"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        (MULTI_AGENT_MODE_OPEN_TAG, MULTI_AGENT_MODE_CLOSE_TAG)
    }

    fn body(&self) -> String {
        match &self.multi_agent_mode {
            MultiAgentMode::Custom(hint_text) => hint_text.clone(),
            MultiAgentMode::ExplicitRequestOnly => {
                EXPLICIT_REQUEST_ONLY_MULTI_AGENT_MODE_TEXT.to_string()
            }
            MultiAgentMode::Balanced => BALANCED_MULTI_AGENT_MODE_TEXT.to_string(),
            MultiAgentMode::Proactive => PROACTIVE_MULTI_AGENT_MODE_TEXT.to_string(),
        }
    }
}
