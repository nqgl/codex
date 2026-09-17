//! Push-to-talk transcription for the composer.

use super::*;
use crate::dictation_audio::DictationCapture;
use crate::dictation_audio::send_transcription_end;
use codex_app_server_protocol::ThreadRealtimeAudioChunk;
use codex_app_server_protocol::ThreadRealtimeClosedNotification;
use codex_app_server_protocol::ThreadRealtimeErrorNotification;
use codex_app_server_protocol::ThreadRealtimeStartedNotification;
use codex_app_server_protocol::ThreadRealtimeTranscriptDeltaNotification;
use codex_app_server_protocol::ThreadRealtimeTranscriptDoneNotification;
use std::collections::VecDeque;
use std::time::Duration;

const DICTATION_KEY: KeyCode = KeyCode::F(8);
const HOLD_TO_DICTATE_KEY: KeyCode = KeyCode::Char(' ');
// The account-backed streaming service allows up to eight seconds to flush its final words.
const TRANSCRIPT_TAIL_WAIT: Duration = Duration::from_secs(/*secs*/ 10);
const MAX_BUFFERED_AUDIO_SAMPLES: u32 = 24_000 * 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum DictationPhase {
    #[default]
    Idle,
    Connecting,
    Listening,
    Finishing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DictationTrigger {
    FunctionKey,
    HoldSpace,
}

impl DictationTrigger {
    fn label(self) -> &'static str {
        match self {
            Self::FunctionKey => "F8",
            Self::HoldSpace => "Space",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DictationKeyAction {
    Unhandled,
    Handled,
    InsertSpace,
    InsertSpaceAndContinue,
    Start(DictationTrigger),
    Stop,
}

pub(super) struct DictationState {
    pub(super) phase: DictationPhase,
    capture: Option<DictationCapture>,
    started: bool,
    audio_committed: bool,
    buffered_audio: VecDeque<ThreadRealtimeAudioChunk>,
    buffered_audio_samples: u32,
    transcript: String,
    partial_transcript: String,
    detected_audio_signal: bool,
    generation: u64,
    hold_space_enabled: bool,
    space_hold_armed: bool,
    trigger: Option<DictationTrigger>,
}

impl DictationState {
    pub(super) fn new(hold_space_enabled: bool) -> Self {
        Self {
            phase: DictationPhase::Idle,
            capture: None,
            started: false,
            audio_committed: false,
            buffered_audio: VecDeque::new(),
            buffered_audio_samples: 0,
            transcript: String::new(),
            partial_transcript: String::new(),
            detected_audio_signal: false,
            generation: 0,
            hold_space_enabled,
            space_hold_armed: false,
            trigger: None,
        }
    }

    fn is_active(&self) -> bool {
        !matches!(self.phase, DictationPhase::Idle)
    }

    fn append_completed_transcript(&mut self, transcript: &str) {
        self.partial_transcript.clear();
        append_transcript_segment(&mut self.transcript, transcript);
    }

    fn take_transcript(&mut self) -> String {
        append_transcript_segment(&mut self.transcript, &self.partial_transcript);
        self.partial_transcript.clear();
        std::mem::take(&mut self.transcript)
    }

    fn buffer_audio(&mut self, frame: ThreadRealtimeAudioChunk) {
        let samples = frame
            .samples_per_channel
            .unwrap_or(frame.sample_rate / 10)
            .max(1);
        if samples > MAX_BUFFERED_AUDIO_SAMPLES {
            return;
        }
        while self.buffered_audio_samples.saturating_add(samples) > MAX_BUFFERED_AUDIO_SAMPLES {
            let Some(removed) = self.buffered_audio.pop_front() else {
                break;
            };
            self.buffered_audio_samples = self.buffered_audio_samples.saturating_sub(
                removed
                    .samples_per_channel
                    .unwrap_or(removed.sample_rate / 10)
                    .max(1),
            );
        }
        self.buffered_audio_samples = self.buffered_audio_samples.saturating_add(samples);
        self.buffered_audio.push_back(frame);
    }

    fn take_buffered_audio(&mut self) -> VecDeque<ThreadRealtimeAudioChunk> {
        self.buffered_audio_samples = 0;
        std::mem::take(&mut self.buffered_audio)
    }

    fn key_label(&self) -> &'static str {
        self.trigger
            .unwrap_or(DictationTrigger::FunctionKey)
            .label()
    }

    fn handle_key_event(
        &mut self,
        key_event: KeyEvent,
        can_arm_space_hold: bool,
    ) -> DictationKeyAction {
        if self.space_hold_armed
            && key_event.code != HOLD_TO_DICTATE_KEY
            && matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat)
        {
            self.space_hold_armed = false;
            return DictationKeyAction::InsertSpaceAndContinue;
        }

        if key_event.code == DICTATION_KEY {
            return match (self.phase, key_event.kind) {
                (DictationPhase::Idle, KeyEventKind::Press) => {
                    DictationKeyAction::Start(DictationTrigger::FunctionKey)
                }
                (
                    DictationPhase::Connecting | DictationPhase::Listening,
                    KeyEventKind::Press | KeyEventKind::Release,
                ) => DictationKeyAction::Stop,
                (
                    DictationPhase::Idle
                    | DictationPhase::Connecting
                    | DictationPhase::Listening
                    | DictationPhase::Finishing,
                    KeyEventKind::Press | KeyEventKind::Repeat | KeyEventKind::Release,
                ) => DictationKeyAction::Handled,
            };
        }

        if key_event.code != HOLD_TO_DICTATE_KEY
            || !key_event.modifiers.is_empty()
            || !self.hold_space_enabled
        {
            return DictationKeyAction::Unhandled;
        }

        if self.trigger == Some(DictationTrigger::HoldSpace) && self.is_active() {
            return match (self.phase, key_event.kind) {
                (DictationPhase::Connecting | DictationPhase::Listening, KeyEventKind::Release) => {
                    DictationKeyAction::Stop
                }
                (
                    DictationPhase::Idle
                    | DictationPhase::Connecting
                    | DictationPhase::Listening
                    | DictationPhase::Finishing,
                    KeyEventKind::Press | KeyEventKind::Repeat | KeyEventKind::Release,
                ) => DictationKeyAction::Handled,
            };
        }

        if self.space_hold_armed {
            self.space_hold_armed = false;
            return match key_event.kind {
                KeyEventKind::Press | KeyEventKind::Repeat => {
                    DictationKeyAction::Start(DictationTrigger::HoldSpace)
                }
                KeyEventKind::Release => DictationKeyAction::InsertSpace,
            };
        }

        if can_arm_space_hold && key_event.kind == KeyEventKind::Press {
            self.space_hold_armed = true;
            return DictationKeyAction::Handled;
        }

        DictationKeyAction::Unhandled
    }
}

fn append_transcript_segment(transcript: &mut String, segment: &str) {
    let segment = segment.trim();
    if segment.is_empty() {
        return;
    }
    if !transcript.is_empty() && !transcript.ends_with(char::is_whitespace) {
        transcript.push(' ');
    }
    transcript.push_str(segment);
}

impl ChatWidget {
    pub(super) fn handle_dictation_key_event(&mut self, key_event: KeyEvent) -> bool {
        if !self.dictation.is_active() && self.realtime_conversation_is_running() {
            self.dictation.space_hold_armed = false;
            return false;
        }
        // The expanded question editor owns text entry. Keep an existing capture's release
        // handling ahead of it, but do not arm/start microphone capture from question input.
        if !self.dictation.is_active()
            && self
                .bottom_pane
                .questions
                .as_ref()
                .is_some_and(|questions| questions.expanded)
        {
            self.dictation.space_hold_armed = false;
            return false;
        }
        if !self.dictation.is_active()
            && (self.chat_keymap.next_permission_mode.is_pressed(key_event)
                || self
                    .chat_keymap
                    .previous_permission_mode
                    .is_pressed(key_event))
        {
            return false;
        }
        if key_event.code == DICTATION_KEY
            && !self.dictation.is_active()
            && !self.bottom_pane.no_modal_or_popup_active()
        {
            return false;
        }

        let can_arm_space_hold = !self.dictation.is_active()
            && self.bottom_pane.no_modal_or_popup_active()
            && self.bottom_pane.composer_is_empty()
            && !self.blocks_direct_input
            && self.config.features.enabled(Feature::RealtimeConversation)
            && self.remote_connection.is_none();
        match self
            .dictation
            .handle_key_event(key_event, can_arm_space_hold)
        {
            DictationKeyAction::Unhandled => false,
            DictationKeyAction::Handled => true,
            DictationKeyAction::InsertSpace => {
                self.bottom_pane.insert_str(" ");
                true
            }
            DictationKeyAction::InsertSpaceAndContinue => {
                self.bottom_pane.insert_str(" ");
                false
            }
            DictationKeyAction::Start(trigger) => {
                self.start_dictation(trigger);
                true
            }
            DictationKeyAction::Stop => {
                self.stop_dictation_capture();
                true
            }
        }
    }

    fn start_dictation(&mut self, trigger: DictationTrigger) {
        if self.blocks_direct_input {
            self.add_error_message(PARENT_OWNED_INPUT_MESSAGE.to_string());
            return;
        }
        if !self.config.features.enabled(Feature::RealtimeConversation) {
            self.add_error_message(
                "Push-to-talk requires the realtime_conversation feature. Start this build with `--enable realtime_conversation`."
                    .to_string(),
            );
            return;
        }
        if self.remote_connection.is_some() {
            self.add_error_message(
                "Push-to-talk is currently available only with the local embedded app-server."
                    .to_string(),
            );
            return;
        }

        self.dictation.generation = self.dictation.generation.wrapping_add(1);
        self.dictation.started = false;
        self.dictation.audio_committed = false;
        self.dictation.buffered_audio.clear();
        self.dictation.buffered_audio_samples = 0;
        self.dictation.transcript.clear();
        self.dictation.partial_transcript.clear();
        self.dictation.detected_audio_signal = false;
        self.dictation.trigger = Some(trigger);
        self.dictation.phase = DictationPhase::Connecting;
        self.bottom_pane.clear_dictation_preview();
        self.bottom_pane.set_footer_hint_override(Some(vec![(
            trigger.label().to_string(),
            "connecting · hold to talk".to_string(),
        )]));
        self.submit_op(AppCommand::dictation_start());

        match DictationCapture::start(self.app_event_tx.clone(), self.dictation.generation) {
            Ok(capture) => self.dictation.capture = Some(capture),
            Err(err) => {
                self.submit_op(AppCommand::dictation_close());
                self.reset_dictation();
                self.add_error_message(format!("Could not start push-to-talk: {err}"));
            }
        }
        self.request_redraw();
    }

    fn stop_dictation_capture(&mut self) {
        if let Some(capture) = self.dictation.capture.take() {
            self.dictation.detected_audio_signal = capture.detected_audio_signal();
        }
        self.dictation.phase = DictationPhase::Finishing;
        send_transcription_end(&self.app_event_tx, self.dictation.generation);
        self.bottom_pane.set_footer_hint_override(Some(vec![(
            self.dictation.key_label().to_string(),
            "transcribing…".to_string(),
        )]));
        self.request_redraw();
    }

    fn schedule_dictation_finalize(&self) {
        let generation = self.dictation.generation;
        let app_event_tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(TRANSCRIPT_TAIL_WAIT).await;
            app_event_tx.send(AppEvent::DictationFinalize { generation });
        });
    }

    pub(crate) fn on_dictation_audio(&mut self, generation: u64, frame: ThreadRealtimeAudioChunk) {
        if self.dictation.generation != generation
            || !self.dictation.is_active()
            || self.dictation.audio_committed
        {
            return;
        }
        if self.dictation.started {
            self.submit_op(AppCommand::dictation_audio(frame));
        } else {
            self.dictation.buffer_audio(frame);
        }
    }

    pub(crate) fn on_dictation_commit(&mut self, generation: u64) {
        if self.dictation.generation != generation
            || !matches!(self.dictation.phase, DictationPhase::Finishing)
            || self.dictation.audio_committed
        {
            return;
        }
        self.dictation.audio_committed = true;
        if self.dictation.started {
            self.submit_op(AppCommand::DictationCommit);
            self.schedule_dictation_finalize();
        }
    }

    pub(super) fn on_dictation_started(
        &mut self,
        _notification: ThreadRealtimeStartedNotification,
    ) {
        if !self.dictation.is_active() || self.dictation.started {
            return;
        }
        self.dictation.started = true;
        for frame in self.dictation.take_buffered_audio() {
            self.submit_op(AppCommand::dictation_audio(frame));
        }
        match self.dictation.phase {
            DictationPhase::Connecting => {
                self.dictation.phase = DictationPhase::Listening;
                self.bottom_pane.set_footer_hint_override(Some(vec![(
                    self.dictation.key_label().to_string(),
                    "listening · release to insert".to_string(),
                )]));
            }
            DictationPhase::Finishing => {
                if self.dictation.audio_committed {
                    self.submit_op(AppCommand::DictationCommit);
                    self.schedule_dictation_finalize();
                }
            }
            DictationPhase::Idle | DictationPhase::Listening => {}
        }
        self.request_redraw();
    }

    pub(crate) fn finalize_dictation(&mut self, generation: u64) {
        if self.dictation.generation != generation
            || !matches!(self.dictation.phase, DictationPhase::Finishing)
        {
            return;
        }

        let transcript = self.dictation.take_transcript();
        let received_only_silence =
            transcript.trim().is_empty() && !self.dictation.detected_audio_signal;
        self.reset_dictation();
        self.submit_op(AppCommand::dictation_close());
        if received_only_silence {
            self.add_error_message(
                "Push-to-talk received only silence. Check that your microphone is connected, powered on, and unmuted."
                    .to_string(),
            );
        } else {
            self.bottom_pane.insert_dictation(&transcript);
        }
    }

    pub(super) fn on_dictation_transcript_delta(
        &mut self,
        notification: ThreadRealtimeTranscriptDeltaNotification,
    ) {
        if self.dictation.is_active() && notification.role == "user" {
            self.dictation
                .partial_transcript
                .push_str(&notification.delta);
            self.bottom_pane.set_dictation_preview(
                &self.dictation.transcript,
                &self.dictation.partial_transcript,
            );
        }
    }

    pub(super) fn on_dictation_transcript_done(
        &mut self,
        notification: ThreadRealtimeTranscriptDoneNotification,
    ) {
        if !self.dictation.is_active() || notification.role != "user" {
            return;
        }
        self.dictation
            .append_completed_transcript(&notification.text);
        self.bottom_pane.set_dictation_preview(
            &self.dictation.transcript,
            &self.dictation.partial_transcript,
        );
        if matches!(self.dictation.phase, DictationPhase::Finishing) {
            self.finalize_dictation(self.dictation.generation);
        }
    }

    pub(super) fn on_dictation_error(&mut self, notification: ThreadRealtimeErrorNotification) {
        if !self.dictation.is_active() {
            return;
        }
        self.submit_op(AppCommand::dictation_close());
        self.reset_dictation();
        self.add_error_message(format!(
            "Push-to-talk transcription failed: {}",
            notification.message
        ));
    }

    pub(super) fn on_dictation_closed(&mut self, notification: ThreadRealtimeClosedNotification) {
        if !self.dictation.is_active() {
            return;
        }
        let transcript = self.dictation.take_transcript();
        self.reset_dictation();
        self.bottom_pane.insert_dictation(&transcript);
        if transcript.trim().is_empty()
            && let Some(reason) = notification.reason
            && reason != "requested"
        {
            self.add_error_message(format!(
                "Push-to-talk closed before transcription: {reason}"
            ));
        }
    }

    fn reset_dictation(&mut self) {
        self.dictation.capture = None;
        self.dictation.phase = DictationPhase::Idle;
        self.dictation.started = false;
        self.dictation.audio_committed = false;
        self.dictation.buffered_audio.clear();
        self.dictation.buffered_audio_samples = 0;
        self.dictation.transcript.clear();
        self.dictation.partial_transcript.clear();
        self.dictation.detected_audio_signal = false;
        self.dictation.space_hold_armed = false;
        self.dictation.trigger = None;
        self.bottom_pane.clear_dictation_preview();
        self.bottom_pane.set_footer_hint_override(None);
        self.request_redraw();
    }
}

#[cfg(test)]
#[path = "dictation_state_tests.rs"]
mod tests;
