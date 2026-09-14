use super::*;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn active_voice_session_owns_keys_without_arming_dictation() {
    let (mut chat, _sender, _rx, _op_rx) =
        crate::chatwidget::tests::make_chatwidget_manual_with_sender().await;
    chat.dictation = DictationState::new(/*hold_space_enabled*/ true);
    chat.realtime_conversation.phase =
        crate::chatwidget::realtime::RealtimeConversationPhase::Active;
    chat.dictation.space_hold_armed = true;
    assert!(!chat.handle_dictation_key_event(key(HOLD_TO_DICTATE_KEY, KeyEventKind::Press)));
    assert!(!chat.handle_dictation_key_event(key(DICTATION_KEY, KeyEventKind::Press)));
    assert_eq!(
        (chat.dictation.phase, chat.dictation.space_hold_armed),
        (DictationPhase::Idle, false)
    );
}

#[tokio::test]
async fn question_editor_owns_space_but_does_not_swallow_capture_release() {
    let (mut chat, _sender, _rx, _op_rx) =
        crate::chatwidget::tests::make_chatwidget_manual_with_sender().await;
    chat.dictation = DictationState::new(/*hold_space_enabled*/ true);
    chat.config
        .features
        .enable(Feature::RealtimeConversation)
        .expect("enable realtime conversation");
    chat.add_async_questions(
        "question",
        &[codex_protocol::items::AsyncUserInputQuestion {
            title: "Choose?".into(),
            options: Some(vec!["One".into(), "Two".into()]),
        }],
    );
    chat.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    chat.dictation.space_hold_armed = true;
    assert!(!chat.handle_dictation_key_event(key(HOLD_TO_DICTATE_KEY, KeyEventKind::Press)));
    assert!(!chat.dictation.space_hold_armed);
    assert_eq!(chat.composer_text_with_pending(), "");

    // No microphone is opened: simulate an already-active stream and exercise its release.
    chat.dictation.phase = DictationPhase::Connecting;
    chat.dictation.trigger = Some(DictationTrigger::HoldSpace);
    chat.handle_key_event(key(HOLD_TO_DICTATE_KEY, KeyEventKind::Release));
    assert_eq!(chat.dictation.phase, DictationPhase::Finishing);
}

fn key(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
    KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)
}

#[test]
fn short_space_tap_inserts_an_ordinary_space() {
    let mut state = DictationState::new(/*hold_space_enabled*/ true);

    assert_eq!(
        state.handle_key_event(key(HOLD_TO_DICTATE_KEY, KeyEventKind::Press), true),
        DictationKeyAction::Handled
    );
    assert_eq!(
        state.handle_key_event(key(HOLD_TO_DICTATE_KEY, KeyEventKind::Release), true),
        DictationKeyAction::InsertSpace
    );
}

#[test]
fn held_space_starts_dictation_on_key_repeat() {
    let mut state = DictationState::new(/*hold_space_enabled*/ true);

    assert_eq!(
        state.handle_key_event(key(HOLD_TO_DICTATE_KEY, KeyEventKind::Press), true),
        DictationKeyAction::Handled
    );
    assert_eq!(
        state.handle_key_event(key(HOLD_TO_DICTATE_KEY, KeyEventKind::Repeat), true),
        DictationKeyAction::Start(DictationTrigger::HoldSpace)
    );
}

#[test]
fn typing_another_key_flushes_an_armed_space_first() {
    let mut state = DictationState::new(/*hold_space_enabled*/ true);
    assert_eq!(
        state.handle_key_event(key(HOLD_TO_DICTATE_KEY, KeyEventKind::Press), true),
        DictationKeyAction::Handled
    );

    assert_eq!(
        state.handle_key_event(key(KeyCode::Char('a'), KeyEventKind::Press), true),
        DictationKeyAction::InsertSpaceAndContinue
    );
}

#[test]
fn space_release_stops_space_triggered_dictation() {
    let mut state = DictationState::new(/*hold_space_enabled*/ true);
    state.phase = DictationPhase::Listening;
    state.trigger = Some(DictationTrigger::HoldSpace);

    assert_eq!(
        state.handle_key_event(key(HOLD_TO_DICTATE_KEY, KeyEventKind::Release), false),
        DictationKeyAction::Stop
    );
}

#[test]
fn space_hold_is_unhandled_when_enhanced_keys_are_unavailable() {
    let mut state = DictationState::new(/*hold_space_enabled*/ false);

    assert_eq!(
        state.handle_key_event(key(HOLD_TO_DICTATE_KEY, KeyEventKind::Press), true),
        DictationKeyAction::Unhandled
    );
}
