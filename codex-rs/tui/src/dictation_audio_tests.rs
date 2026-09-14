use super::convert_pcm16;
use super::note_audio_signal;
use pretty_assertions::assert_eq;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

#[test]
fn transcription_end_queues_padding_before_commit() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let tx = crate::app_event_sender::AppEventSender::new(tx);
    super::send_transcription_end(&tx, /*generation*/ 7);
    let Some(crate::app_event::AppEvent::DictationAudio { generation, frame }) = rx.try_recv().ok()
    else {
        panic!("expected padding audio");
    };
    assert_eq!(
        (generation, frame.sample_rate, frame.samples_per_channel),
        (7, 24_000, Some(14_400))
    );
    assert!(matches!(
        rx.try_recv(),
        Ok(crate::app_event::AppEvent::DictationCommit { generation: 7 })
    ));
    assert!(rx.try_recv().is_err());
}

#[test]
fn downmixes_and_resamples_for_transcription() {
    let input = vec![100, 300, 200, 400, 500, 700, 600, 800];
    let converted = convert_pcm16(
        &input, /*input_sample_rate*/ 48_000, /*input_channels*/ 2,
        /*output_sample_rate*/ 24_000, /*output_channels*/ 1,
    );
    assert_eq!(converted, vec![200, 700]);
}

#[test]
fn detects_nonzero_microphone_samples() {
    let detected_audio_signal = AtomicBool::new(false);

    note_audio_signal(&[0, 0, 0], &detected_audio_signal);
    assert_eq!(detected_audio_signal.load(Ordering::Relaxed), false);

    note_audio_signal(&[0, 1, 0], &detected_audio_signal);
    assert_eq!(detected_audio_signal.load(Ordering::Relaxed), true);
}
