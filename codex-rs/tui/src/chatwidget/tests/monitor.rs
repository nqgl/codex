use super::*;
use crate::monitor::MonitorNotification;
use crate::monitor::MonitorTrust;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn monitor_output_steers_a_running_turn() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.thread_id = Some(ThreadId::new());
    handle_turn_started(&mut chat, "turn-1");

    chat.handle_monitor_notification(MonitorNotification {
        monitor_id: 1,
        name: "agent-comms".to_string(),
        output: "abc123 tagged codex".to_string(),
        omitted_bytes: 0,
        trust: MonitorTrust::Trusted,
    });

    assert_eq!(chat.input_queue.queued_user_messages.len(), 0);
    assert_eq!(chat.input_queue.pending_steers.len(), 1);
    assert_eq!(
        chat.input_queue.pending_steers[0].user_message.text,
        "[Monitor event]\nMonitor: agent-comms\nOutput:\nabc123 tagged codex"
    );
}
