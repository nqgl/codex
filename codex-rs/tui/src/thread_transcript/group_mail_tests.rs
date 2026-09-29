use super::RawReasoningVisibility;
use super::thread_items_to_transcript_cells;
use crate::test_support::PathBufExt;
use crate::test_support::test_path_buf;
use codex_app_server_protocol::ThreadItem;
use codex_protocol::models::MessagePhase;
use pretty_assertions::assert_eq;

#[test]
fn received_mail_survives_transcript_projection() {
    let cwd = test_path_buf("/workspace").abs();
    let cells = thread_items_to_transcript_cells(
        /*thread_id*/ None,
        &cwd,
        vec![ThreadItem::AgentMessage {
            id: "group-mail-received-123".to_string(),
            text: "From alice to bob:\nhello".to_string(),
            phase: Some(MessagePhase::Commentary),
            memory_citation: None,
            delivery: None,
            questions: None,
        }],
        RawReasoningVisibility::Hidden,
        /*config*/ None,
    );
    let rendered = cells
        .into_iter()
        .flat_map(|cell| cell.display_lines(/*width*/ 30))
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    assert_eq!(rendered, vec!["✉ From alice to bob:", "  hello"]);
}
