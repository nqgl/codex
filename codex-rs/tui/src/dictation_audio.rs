//! Microphone capture for composer dictation.
//!
//! The TUI records only while the push-to-talk key is held. Captured audio is converted to the
//! 24 kHz mono PCM16 format expected by the realtime transcription endpoint and forwarded through
//! the ordinary app event channel. There is deliberately no playback path here.

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use base64::Engine;
use codex_app_server_protocol::ThreadRealtimeAudioChunk;
use cpal::traits::DeviceTrait;
use cpal::traits::HostTrait;
use cpal::traits::StreamTrait;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tracing::error;

const TRANSCRIPTION_SAMPLE_RATE: u32 = 24_000;
const TRANSCRIPTION_CHANNELS: u16 = 1;
const TRANSCRIPTION_SILENCE_TAIL_MS: u32 = 600;

pub(crate) struct DictationCapture {
    _stream: cpal::Stream,
    detected_audio_signal: Arc<AtomicBool>,
}

impl DictationCapture {
    pub(crate) fn start(tx: AppEventSender, generation: u64) -> Result<Self, String> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or_else(|| "no microphone input device is available".to_string())?;
        let config = device
            .default_input_config()
            .map_err(|err| format!("failed to get the default microphone format: {err}"))?;
        let sample_rate = config.sample_rate();
        let channels = config.channels();
        let detected_audio_signal = Arc::new(AtomicBool::new(false));
        let stream = build_input_stream(
            &device,
            &config,
            sample_rate,
            channels,
            tx,
            generation,
            Arc::clone(&detected_audio_signal),
        )?;
        stream
            .play()
            .map_err(|err| format!("failed to start microphone capture: {err}"))?;
        Ok(Self {
            _stream: stream,
            detected_audio_signal,
        })
    }

    pub(crate) fn detected_audio_signal(&self) -> bool {
        self.detected_audio_signal.load(Ordering::Relaxed)
    }
}

pub(crate) fn send_transcription_end(tx: &AppEventSender, generation: u64) {
    let samples = TRANSCRIPTION_SAMPLE_RATE * TRANSCRIPTION_SILENCE_TAIL_MS / 1_000;
    send_audio_chunk(
        tx,
        vec![0; samples as usize],
        TRANSCRIPTION_SAMPLE_RATE,
        TRANSCRIPTION_CHANNELS,
        generation,
    );
    tx.send(AppEvent::DictationCommit { generation });
}

fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    sample_rate: u32,
    channels: u16,
    tx: AppEventSender,
    generation: u64,
    detected_audio_signal: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    match config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                (*config).into(),
                move |input: &[f32], _| {
                    let samples: Vec<i16> = input.iter().copied().map(f32_to_i16).collect();
                    note_audio_signal(&samples, &detected_audio_signal);
                    send_audio_chunk(&tx, samples, sample_rate, channels, generation);
                },
                log_input_error,
                None,
            )
            .map_err(|err| format!("failed to open the microphone: {err}")),
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                (*config).into(),
                move |input: &[i16], _| {
                    note_audio_signal(input, &detected_audio_signal);
                    send_audio_chunk(&tx, input.to_vec(), sample_rate, channels, generation);
                },
                log_input_error,
                None,
            )
            .map_err(|err| format!("failed to open the microphone: {err}")),
        cpal::SampleFormat::U16 => device
            .build_input_stream(
                (*config).into(),
                move |input: &[u16], _| {
                    let samples: Vec<i16> = input
                        .iter()
                        .map(|sample| (*sample as i32 - 32_768) as i16)
                        .collect();
                    note_audio_signal(&samples, &detected_audio_signal);
                    send_audio_chunk(&tx, samples, sample_rate, channels, generation);
                },
                log_input_error,
                None,
            )
            .map_err(|err| format!("failed to open the microphone: {err}")),
        _ => Err("the microphone uses an unsupported sample format".to_string()),
    }
}

fn log_input_error(err: cpal::Error) {
    error!("microphone input error: {err}");
}

fn note_audio_signal(samples: &[i16], detected_audio_signal: &AtomicBool) {
    if samples.iter().any(|sample| *sample != 0) {
        detected_audio_signal.store(true, Ordering::Relaxed);
    }
}

fn send_audio_chunk(
    tx: &AppEventSender,
    samples: Vec<i16>,
    sample_rate: u32,
    channels: u16,
    generation: u64,
) {
    if samples.is_empty() || sample_rate == 0 || channels == 0 {
        return;
    }

    let samples = convert_pcm16(
        &samples,
        sample_rate,
        channels,
        TRANSCRIPTION_SAMPLE_RATE,
        TRANSCRIPTION_CHANNELS,
    );
    if samples.is_empty() {
        return;
    }

    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in &samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let samples_per_channel = samples.len() as u32;
    tx.send(AppEvent::DictationAudio {
        generation,
        frame: ThreadRealtimeAudioChunk {
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
            sample_rate: TRANSCRIPTION_SAMPLE_RATE,
            num_channels: TRANSCRIPTION_CHANNELS,
            samples_per_channel: Some(samples_per_channel),
            item_id: None,
        },
    });
}

fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

fn convert_pcm16(
    input: &[i16],
    input_sample_rate: u32,
    input_channels: u16,
    output_sample_rate: u32,
    output_channels: u16,
) -> Vec<i16> {
    if input.is_empty()
        || input_sample_rate == 0
        || input_channels == 0
        || output_sample_rate == 0
        || output_channels == 0
    {
        return Vec::new();
    }

    let input_channels = usize::from(input_channels);
    let output_channels = usize::from(output_channels);
    let input_frames = input.len() / input_channels;
    if input_frames == 0 {
        return Vec::new();
    }

    let output_frames = if input_sample_rate == output_sample_rate {
        input_frames
    } else {
        ((input_frames as u64 * u64::from(output_sample_rate)) / u64::from(input_sample_rate))
            .max(1) as usize
    };
    let mut output = Vec::with_capacity(output_frames.saturating_mul(output_channels));
    for output_frame_index in 0..output_frames {
        let input_frame_index = if output_frames <= 1 || input_frames <= 1 {
            0
        } else {
            output_frame_index * (input_frames - 1) / (output_frames - 1)
        };
        let start = input_frame_index * input_channels;
        let frame = &input[start..start + input_channels];
        match (input_channels, output_channels) {
            (1, 1) => output.push(frame[0]),
            (1, count) => output.extend(std::iter::repeat_n(frame[0], count)),
            (count, 1) => {
                let sum: i32 = frame.iter().map(|sample| i32::from(*sample)).sum();
                output.push((sum / count as i32) as i16);
            }
            (input_count, output_count) if input_count == output_count => {
                output.extend_from_slice(frame);
            }
            (input_count, output_count) if input_count > output_count => {
                output.extend_from_slice(&frame[..output_count]);
            }
            (input_count, output_count) => {
                output.extend_from_slice(frame);
                output.extend(std::iter::repeat_n(
                    frame.last().copied().unwrap_or_default(),
                    output_count - input_count,
                ));
            }
        }
    }
    output
}

#[cfg(test)]
#[path = "dictation_audio_tests.rs"]
mod tests;
