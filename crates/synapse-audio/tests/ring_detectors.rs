use std::sync::{Arc, Mutex, MutexGuard};

use synapse_audio::{
    AudioEventSink, AudioRing,
    detectors::{DetectorProcessor, SharedDetectorState},
    ring::AudioFormat,
};
use synapse_core::Event;

#[test]
fn ring_tail_preserves_expected_frame_and_byte_counts() -> Result<(), Box<dyn std::error::Error>> {
    let ring = AudioRing::new(5);
    let format = AudioFormat {
        sample_rate_hz: 48_000,
        channels: 2,
    };
    ring.set_format(format);
    ring.push_interleaved(&vec![0.5; 48_000 * 2 * 3]);

    let window = ring.tail_seconds(2.0)?;

    assert_eq!(window.frames, 96_000);
    assert_eq!(window.samples.len(), 192_000);
    assert_eq!(window.pcm_i16_le().len(), 384_000);
    assert!(window.rms_db > -7.0);
    Ok(())
}

#[test]
fn ring_rejects_over_capacity_tail() {
    let ring = AudioRing::new(5);
    let error = match ring.tail_seconds(6.0) {
        Ok(window) => panic!("expected over-capacity tail to fail, got {window:?}"),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        synapse_core::error_codes::AUDIO_LOOPBACK_INIT_FAILED
    );
}

#[test]
fn detectors_emit_loud_and_speech_events_for_synthetic_audio() {
    let events = Arc::new(Mutex::new(Vec::<Event>::new()));
    let sink_events = Arc::clone(&events);
    let sink: AudioEventSink = Arc::new(move |event| {
        lock_events(&sink_events).push(event);
    });
    let state = SharedDetectorState::default();
    let mut detector = DetectorProcessor::new(state.clone(), sink);
    let format = AudioFormat {
        sample_rate_hz: 48_000,
        channels: 2,
    };

    detector.process(&vec![0.0; 480 * 2], format);
    detector.process(&vec![0.9; 480 * 2], format);
    for _ in 0..50 {
        detector.process(&vec![0.0; 480 * 2], format);
    }

    let kinds = lock_events(&events)
        .iter()
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();
    assert!(kinds.iter().any(|kind| kind == "loud_transient"));
    assert!(kinds.iter().any(|kind| kind == "speech_started"));
    assert!(kinds.iter().any(|kind| kind == "speech_ended"));
    assert!(!state.snapshot().speech_active);
}

fn lock_events(events: &Mutex<Vec<Event>>) -> MutexGuard<'_, Vec<Event>> {
    match events.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
