use super::*;
use crate::chatwidget::directory_watch::merge_watch_messages;
use crate::directory_watch::DirectoryWatchNotification;
use crate::directory_watch::DirectoryWatchTrust;
use crate::directory_watch::DirectoryWatchUrgency;
use crate::directory_watch::WATCH_NOTIFICATION_PREFIX;
use pretty_assertions::assert_eq;

#[test]
fn queued_watch_messages_merge_but_remain_bounded() {
    let first = format!("{WATCH_NOTIFICATION_PREFIX}\nfirst");
    let second = format!("{WATCH_NOTIFICATION_PREFIX}\nsecond");
    assert_eq!(
        merge_watch_messages(&first, &second),
        format!("{first}\n\n{second}")
    );

    let oversized = format!("{WATCH_NOTIFICATION_PREFIX}\n{}", "x".repeat(/*n*/ 12_000));
    assert_eq!(
        merge_watch_messages(&oversized, &second),
        format!("{second}\n\n[Earlier queued directory watcher events were omitted.]")
    );
}

#[tokio::test]
async fn low_priority_events_batch_in_the_follow_up_queue() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    handle_turn_started(&mut chat, "turn-1");

    chat.handle_directory_watch_notification(notification(
        "/workspace/first",
        DirectoryWatchUrgency::Low,
    ));
    chat.queue_user_message(UserMessage::from("user queued message"));
    chat.handle_directory_watch_notification(notification(
        "/workspace/second",
        DirectoryWatchUrgency::Low,
    ));

    assert_eq!(chat.input_queue.queued_user_messages.len(), 2);
    let watcher_message = &chat.input_queue.queued_user_messages[0].user_message.text;
    assert!(watcher_message.contains("/workspace/first"));
    assert!(watcher_message.contains("/workspace/second"));
}

#[tokio::test]
async fn high_priority_event_steers_a_running_turn() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.thread_id = Some(ThreadId::new());
    handle_turn_started(&mut chat, "turn-1");

    chat.handle_directory_watch_notification(notification(
        "/workspace/high",
        DirectoryWatchUrgency::High,
    ));

    assert_eq!(chat.input_queue.queued_user_messages.len(), 0);
    assert_eq!(chat.input_queue.pending_steers.len(), 1);
    assert!(
        chat.input_queue.pending_steers[0]
            .user_message
            .text
            .contains("/workspace/high")
    );
}

fn notification(root: &str, urgency: DirectoryWatchUrgency) -> DirectoryWatchNotification {
    let root = PathBuf::from(root);
    DirectoryWatchNotification {
        watch_id: 1,
        changed_paths: vec![root.join("notes.md")],
        root,
        relevant_path_count: 1,
        uninspected_path_count: 0,
        commit: None,
        matched_tags: Vec::new(),
        trust: DirectoryWatchTrust::Trusted,
        urgency,
    }
}
