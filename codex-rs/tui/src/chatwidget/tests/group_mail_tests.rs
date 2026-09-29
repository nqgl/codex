use super::*;
use codex_protocol::models::MessagePhase;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn received_mail_renders_full_body_in_the_live_chat() {
    let (mut chat, mut events, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.handle_thread_item(
        ThreadItem::AgentMessage {
            id: "group-mail-received-123".to_string(),
            text: "From alice to bob:\nFirst line\nSecond line".to_string(),
            phase: Some(MessagePhase::Commentary),
            memory_citation: None,
            delivery: None,
            questions: None,
        },
        "turn".to_string(),
        ThreadItemRenderSource::Live,
    );
    let mail = drain_insert_history(&mut events)
        .into_iter()
        .flatten()
        .map(|line| line.to_string())
        .filter(|line| {
            line.starts_with('✉') || line.starts_with("  First") || line.starts_with("  Second")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        mail,
        vec!["✉ From alice to bob:", "  First line", "  Second line"]
    );
    assert_eq!(chat.transcript.last_completed_agent_message, None);
}
