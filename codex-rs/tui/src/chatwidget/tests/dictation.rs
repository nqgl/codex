use super::*;
use crate::chatwidget::dictation::DictationPhase;
use codex_app_server_protocol::ThreadRealtimeAudioChunk;
use codex_app_server_protocol::ThreadRealtimeErrorNotification;
use codex_app_server_protocol::ThreadRealtimeStartedNotification;
use codex_app_server_protocol::ThreadRealtimeTranscriptDeltaNotification;
use codex_app_server_protocol::ThreadRealtimeTranscriptDoneNotification;
use codex_protocol::protocol::RealtimeConversationVersion;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;

#[tokio::test]
async fn voice_start_waits_for_active_dictation() {
    let (mut chat, mut rx, mut op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.dictation.phase = DictationPhase::Finishing;
    chat.toggle_realtime_conversation();
    assert!(!chat.realtime_conversation_is_running());
    assert!(op_rx.try_recv().is_err());
    let cells = drain_insert_history(&mut rx);
    let lines = cells
        .iter()
        .flatten()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!("voice_start_waits_for_dictation", lines);
}

#[tokio::test]
async fn push_to_talk_connecting_footer_snapshot() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.show_welcome_banner = false;
    chat.dictation.phase = DictationPhase::Connecting;
    chat.bottom_pane.set_footer_hint_override(Some(vec![(
        "F8".to_string(),
        "connecting · hold to talk".to_string(),
    )]));

    let width = 80;
    let height = chat.desired_height(width);
    let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| chat.render(frame.area(), frame.buffer_mut()))
        .expect("draw push-to-talk footer");

    assert_chatwidget_snapshot!(
        "push_to_talk_connecting_footer",
        normalized_backend_snapshot(terminal.backend())
    );
}

#[tokio::test]
async fn push_to_talk_listening_footer_snapshot() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.show_welcome_banner = false;
    chat.dictation.phase = DictationPhase::Listening;
    chat.bottom_pane.set_footer_hint_override(Some(vec![(
        "F8".to_string(),
        "listening · release to insert".to_string(),
    )]));

    let width = 80;
    let height = chat.desired_height(width);
    let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| chat.render(frame.area(), frame.buffer_mut()))
        .expect("draw push-to-talk footer");

    assert_chatwidget_snapshot!(
        "push_to_talk_listening_footer",
        normalized_backend_snapshot(terminal.backend())
    );
}

#[tokio::test]
async fn push_to_talk_streaming_preview_snapshot() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.show_welcome_banner = false;
    chat.dictation.phase = DictationPhase::Listening;
    chat.bottom_pane.set_footer_hint_override(Some(vec![(
        "Space".to_string(),
        "listening · release to insert".to_string(),
    )]));
    let thread_id = ThreadId::new().to_string();
    chat.on_dictation_transcript_done(ThreadRealtimeTranscriptDoneNotification {
        thread_id: thread_id.clone(),
        role: "user".to_string(),
        text: "final words".to_string(),
    });
    chat.on_dictation_transcript_delta(ThreadRealtimeTranscriptDeltaNotification {
        thread_id,
        role: "user".to_string(),
        delta: "still changing".to_string(),
    });

    let width = 80;
    let height = chat.desired_height(width);
    let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| chat.render(frame.area(), frame.buffer_mut()))
        .expect("draw streaming push-to-talk preview");

    assert_chatwidget_snapshot!(
        "push_to_talk_streaming_preview",
        normalized_backend_snapshot(terminal.backend())
    );
    let buffer = terminal.backend().buffer();
    assert!(
        !buffer
            .cell((2, 2))
            .expect("completed transcript cell")
            .style()
            .add_modifier
            .contains(Modifier::DIM)
    );
    assert!(
        buffer
            .cell((14, 2))
            .expect("partial transcript cell")
            .style()
            .add_modifier
            .contains(Modifier::DIM)
    );
}

#[tokio::test]
async fn dictation_audio_waits_for_realtime_started() {
    let (mut chat, _rx, mut op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.dictation.phase = DictationPhase::Connecting;
    let frame = ThreadRealtimeAudioChunk {
        data: "AQI=".to_string(),
        sample_rate: 24_000,
        num_channels: 1,
        samples_per_channel: Some(1),
        item_id: None,
    };

    chat.on_dictation_audio(/*generation*/ 0, frame.clone());
    assert_matches!(
        op_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    );

    chat.on_dictation_started(ThreadRealtimeStartedNotification {
        thread_id: ThreadId::new().to_string(),
        realtime_session_id: Some("realtime-session".to_string()),
        version: RealtimeConversationVersion::V2,
    });

    assert_eq!(op_rx.try_recv(), Ok(Op::DictationAudio(frame)));
    assert_eq!(chat.dictation.phase, DictationPhase::Listening);
}

#[tokio::test]
async fn dictation_release_before_connection_commits_after_buffered_audio() {
    let (mut chat, _rx, mut op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.dictation.phase = DictationPhase::Finishing;
    let frame = ThreadRealtimeAudioChunk {
        data: "AQI=".to_string(),
        sample_rate: 24_000,
        num_channels: 1,
        samples_per_channel: Some(1),
        item_id: None,
    };
    chat.on_dictation_audio(/*generation*/ 0, frame.clone());
    chat.on_dictation_commit(/*generation*/ 0);
    assert!(op_rx.try_recv().is_err());
    chat.on_dictation_started(ThreadRealtimeStartedNotification {
        thread_id: ThreadId::new().to_string(),
        realtime_session_id: Some("dictation".to_string()),
        version: RealtimeConversationVersion::V2,
    });
    chat.on_dictation_commit(/*generation*/ 0);
    chat.on_dictation_audio(/*generation*/ 0, frame.clone());
    assert_eq!(
        vec![op_rx.try_recv().unwrap(), op_rx.try_recv().unwrap()],
        vec![Op::DictationAudio(frame), Op::DictationCommit]
    );
    assert!(op_rx.try_recv().is_err());
}

#[tokio::test]
async fn dictation_commit_waits_for_final_transcript_before_insertion() {
    let (mut chat, _rx, mut op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.dictation.phase = DictationPhase::Connecting;
    chat.on_dictation_started(ThreadRealtimeStartedNotification {
        thread_id: ThreadId::new().to_string(),
        realtime_session_id: Some("dictation".to_string()),
        version: RealtimeConversationVersion::V2,
    });
    chat.dictation.phase = DictationPhase::Finishing;
    chat.on_dictation_commit(/*generation*/ 0);
    assert_eq!(op_rx.try_recv().unwrap(), Op::DictationCommit);
    assert_eq!(chat.bottom_pane.composer_text(), "");
    chat.on_dictation_transcript_done(ThreadRealtimeTranscriptDoneNotification {
        thread_id: ThreadId::new().to_string(),
        role: "user".to_string(),
        text: "Final live transcript.".to_string(),
    });
    assert_eq!(chat.bottom_pane.composer_text(), "Final live transcript.");
    assert_eq!(op_rx.try_recv().unwrap(), Op::DictationClose);
    chat.show_welcome_banner = false;
    let height = chat.desired_height(/*width*/ 80);
    let mut terminal =
        ratatui::Terminal::new(TestBackend::new(/*width*/ 80, height)).expect("terminal");
    terminal
        .draw(|frame| chat.render(frame.area(), frame.buffer_mut()))
        .expect("draw");
    assert_chatwidget_snapshot!(
        "live_dictation_final_in_composer",
        normalized_backend_snapshot(terminal.backend())
    );
}

#[tokio::test]
async fn failed_dictation_drops_buffered_and_stale_audio() {
    let (mut chat, _rx, mut op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.dictation.phase = DictationPhase::Connecting;
    let frame = ThreadRealtimeAudioChunk {
        data: "AQI=".to_string(),
        sample_rate: 24_000,
        num_channels: 1,
        samples_per_channel: Some(1),
        item_id: None,
    };
    chat.on_dictation_audio(/*generation*/ 0, frame.clone());

    chat.on_dictation_error(ThreadRealtimeErrorNotification {
        thread_id: ThreadId::new().to_string(),
        message: "connection closed".to_string(),
    });
    chat.on_dictation_audio(/*generation*/ 0, frame);

    assert_eq!(op_rx.try_recv(), Ok(Op::DictationClose));
    assert_matches!(
        op_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    );
}

#[tokio::test]
async fn completed_dictation_is_inserted_without_submitting() {
    let (mut chat, _rx, mut op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.dictation.phase = DictationPhase::Finishing;

    chat.on_dictation_transcript_done(ThreadRealtimeTranscriptDoneNotification {
        thread_id: ThreadId::new().to_string(),
        role: "user".to_string(),
        text: "hello from the microphone".to_string(),
    });

    assert_eq!(
        chat.bottom_pane.composer_text(),
        "hello from the microphone"
    );
    assert_matches!(op_rx.try_recv(), Ok(Op::DictationClose));
}

#[tokio::test]
async fn silent_microphone_reports_a_clear_error() {
    let (mut chat, mut rx, mut op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.dictation.phase = DictationPhase::Finishing;

    chat.finalize_dictation(/*generation*/ 0);

    assert_matches!(op_rx.try_recv(), Ok(Op::DictationClose));
    let [cell]: [_; 1] = drain_insert_history(&mut rx)
        .try_into()
        .expect("one silence error");
    insta::assert_snapshot!(lines_to_single_string(&cell), @r"■ Push-to-talk received only silence. Check that your microphone is connected, powered on, and unmuted.
");
}

#[tokio::test]
async fn dictation_preserves_multiple_vad_segments() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.dictation.phase = DictationPhase::Listening;
    let thread_id = ThreadId::new().to_string();

    chat.on_dictation_transcript_done(ThreadRealtimeTranscriptDoneNotification {
        thread_id: thread_id.clone(),
        role: "user".to_string(),
        text: "first thought".to_string(),
    });
    chat.on_dictation_transcript_delta(ThreadRealtimeTranscriptDeltaNotification {
        thread_id,
        role: "user".to_string(),
        delta: "then the rest".to_string(),
    });
    chat.dictation.phase = DictationPhase::Finishing;
    chat.finalize_dictation(/*generation*/ 0);

    assert_eq!(
        chat.bottom_pane.composer_text(),
        "first thought then the rest"
    );
}
