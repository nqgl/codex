use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn up_recalls_tab_queue_but_preserves_nonempty_draft() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.thread_id = Some(ThreadId::new());
    handle_turn_started(&mut chat, "turn");
    chat.handle_paste("queued draft".into());
    chat.handle_key_event(KeyCode::Tab.into());
    chat.handle_paste("work in progress".into());
    chat.handle_key_event(KeyCode::Up.into());
    assert_eq!(chat.composer_text_with_pending(), "work in progress");
    assert_eq!(chat.input_queue.queued_user_messages.len(), 1);
    chat.bottom_pane
        .set_composer_text(String::new(), Vec::new(), Vec::new());
    chat.handle_key_event(KeyCode::Up.into());
    assert_eq!(chat.composer_text_with_pending(), "queued draft");
    assert!(chat.input_queue.queued_user_messages.is_empty());
    assert_no_submit_op(&mut ops);
}

#[tokio::test]
async fn repeatedly_recalling_and_requeueing_keeps_one_message() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.thread_id = Some(ThreadId::new());
    handle_turn_started(&mut chat, "turn");
    chat.handle_paste("repeat me".into());
    chat.handle_key_event(KeyCode::Tab.into());
    for _ in 0..3 {
        chat.handle_key_event(KeyCode::Up.into());
        assert!(chat.input_queue.queued_user_messages.is_empty());
        assert_eq!(chat.composer_text_with_pending(), "repeat me");
        chat.handle_key_event(KeyCode::Tab.into());
        assert_eq!(chat.input_queue.queued_user_messages.len(), 1);
    }
    assert_no_submit_op(&mut ops);
}

#[tokio::test]
async fn enter_recall_waits_for_server_confirmation_and_preserves_new_typing() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(/*model_override*/ None).await;
    let thread_id = ThreadId::new();
    chat.thread_id = Some(thread_id);
    handle_turn_started(&mut chat, "turn");
    chat.handle_paste("steer message".into());
    chat.handle_key_event(KeyCode::Enter.into());
    let Op::UserTurn {
        client_user_message_id: client_id,
        ..
    } = ops.try_recv().unwrap()
    else {
        panic!("expected steer submission");
    };
    chat.handle_key_event(KeyCode::Up.into());
    assert_eq!(
        ops.try_recv().unwrap(),
        Op::RecallPendingSteer {
            thread_id,
            expected_turn_id: "turn".into(),
            client_id: client_id.clone(),
        }
    );
    chat.handle_key_event(KeyCode::Up.into());
    assert_no_submit_op(&mut ops);
    assert_eq!(chat.composer_text_with_pending(), "");
    assert_eq!(chat.input_queue.pending_steers.len(), 1);
    chat.handle_paste("new draft".into());
    chat.finish_pending_input_recall(&client_id, Ok(true));
    assert!(chat.input_queue.pending_steers.is_empty());
    assert_eq!(
        chat.composer_text_with_pending(),
        "steer message\nnew draft"
    );
    assert_chatwidget_snapshot!(
        "pending_steer_recalled_into_draft",
        render_bottom_popup(&chat, /*width*/ 80)
    );
    assert_no_submit_op(&mut ops);
}

#[tokio::test]
async fn mixed_enter_and_tab_recall_uses_latest_submission() {
    for keys in [
        [KeyCode::Tab, KeyCode::Enter],
        [KeyCode::Enter, KeyCode::Tab],
    ] {
        let (mut chat, _rx, mut ops) = make_chatwidget_manual(/*model_override*/ None).await;
        chat.thread_id = Some(ThreadId::new());
        handle_turn_started(&mut chat, "turn");
        for (text, key) in [("older", keys[0]), ("newer", keys[1])] {
            chat.handle_paste(text.into());
            chat.handle_key_event(key.into());
        }
        while ops.try_recv().is_ok() {}
        chat.handle_key_event(KeyCode::Up.into());
        if keys[1] == KeyCode::Enter {
            let Op::RecallPendingSteer { client_id, .. } = ops.try_recv().unwrap() else {
                panic!("expected recall");
            };
            chat.finish_pending_input_recall(&client_id, Ok(true));
        }
        assert_eq!(chat.composer_text_with_pending(), "newer");
        assert_eq!(
            chat.input_queue.queued_user_messages.len() + chat.input_queue.pending_steers.len(),
            1
        );
        assert_no_submit_op(&mut ops);
    }
}

#[tokio::test]
async fn consumed_or_failed_steer_recall_does_not_duplicate_input() {
    for (name, result) in [
        ("consumed", Ok(false)),
        ("failed", Err("method unavailable".to_string())),
    ] {
        let (mut chat, mut rx, mut ops) = make_chatwidget_manual(/*model_override*/ None).await;
        chat.thread_id = Some(ThreadId::new());
        handle_turn_started(&mut chat, "turn");
        chat.handle_paste("pending".into());
        chat.handle_key_event(KeyCode::Enter.into());
        let Op::UserTurn {
            client_user_message_id: client_id,
            ..
        } = ops.try_recv().unwrap()
        else {
            panic!("expected steer");
        };
        while rx.try_recv().is_ok() {}
        chat.handle_key_event(KeyCode::Up.into());
        ops.try_recv().unwrap();
        chat.finish_pending_input_recall(&client_id, result);
        assert_eq!(chat.composer_text_with_pending(), "");
        assert_eq!(chat.input_queue.pending_steers.len(), 1);
        assert!(chat.input_queue.recalling_steer.is_none());
        let lines = drain_insert_history(&mut rx)
            .into_iter()
            .flatten()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        insta::assert_debug_snapshot!(format!("pending_steer_recall_{name}"), lines);
        assert_no_submit_op(&mut ops);
    }
}

#[tokio::test]
async fn alternate_recall_binding_preserves_images_and_mentions() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.thread_id = Some(ThreadId::new());
    handle_turn_started(&mut chat, "turn");
    let message = UserMessage {
        text: "[Image #2] examine $file".into(),
        text_elements: vec![TextElement::new((0..10).into(), Some("[Image #2]".into()))],
        local_images: vec![LocalImageAttachment {
            placeholder: "[Image #2]".into(),
            path: PathBuf::from("/tmp/recall.png"),
        }],
        remote_image_urls: vec!["https://example.com/remote.png".into()],
        mention_bindings: vec![MentionBinding {
            sigil: '$',
            mention: "file".into(),
            path: "/tmp/skills/file/SKILL.md".into(),
        }],
    };
    chat.restore_user_message_to_composer(message.clone());
    let expected = chat.bottom_pane.composer_draft_snapshot();
    chat.bottom_pane
        .set_composer_text(String::new(), Vec::new(), Vec::new());
    chat.set_remote_image_urls(Vec::new());
    chat.input_queue.pending_steers.push_back(PendingSteer {
        user_message: message,
        ..pending_steer("attachments")
    });
    chat.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    assert!(matches!(
        ops.try_recv().unwrap(),
        Op::RecallPendingSteer { .. }
    ));
    chat.finish_pending_input_recall("test-submission", Ok(true));
    let restored = chat.bottom_pane.composer_draft_snapshot();
    assert_eq!(
        (
            restored.text,
            restored.cursor,
            restored.text_elements,
            restored.local_images,
            restored.remote_image_urls,
            restored.mention_bindings,
            restored.pending_pastes
        ),
        (
            expected.text,
            expected.cursor,
            expected.text_elements,
            expected.local_images,
            expected.remote_image_urls,
            expected.mention_bindings,
            expected.pending_pastes
        ),
    );
    assert_no_submit_op(&mut ops);
}

#[tokio::test]
async fn up_in_history_search_does_not_recall_pending_input() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.thread_id = Some(ThreadId::new());
    handle_turn_started(&mut chat, "turn");
    chat.input_queue
        .pending_steers
        .push_back(pending_steer("pending"));
    chat.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    chat.handle_key_event(KeyCode::Up.into());
    assert_eq!(chat.input_queue.pending_steers.len(), 1);
    assert!(chat.input_queue.recalling_steer.is_none());
    assert_no_submit_op(&mut ops);
}
