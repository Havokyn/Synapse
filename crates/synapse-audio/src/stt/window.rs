use crate::AudioWindow;

const WHISPER_SAMPLE_RATE_HZ: u32 = 16_000;

pub(super) fn wav_bytes_from_window(window: &AudioWindow) -> Vec<u8> {
    let mono = mono_16khz(window);
    let data_len = mono.len().saturating_mul(2);
    let data_len_u32 = u32::try_from(data_len).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36_u32.saturating_add(data_len_u32)).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&WHISPER_SAMPLE_RATE_HZ.to_le_bytes());
    out.extend_from_slice(&(WHISPER_SAMPLE_RATE_HZ * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len_u32.to_le_bytes());
    for sample in mono {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn mono_16khz(window: &AudioWindow) -> Vec<i16> {
    let source_rate = window.format.sample_rate_hz.max(1);
    let channels = usize::from(window.format.channels.max(1));
    let target_frames = (window
        .frames
        .saturating_mul(WHISPER_SAMPLE_RATE_HZ as usize)
        + source_rate as usize / 2)
        / source_rate as usize;
    let mut out = Vec::with_capacity(target_frames);
    for idx in 0..target_frames {
        let source_frame =
            idx.saturating_mul(source_rate as usize) / WHISPER_SAMPLE_RATE_HZ as usize;
        let start = source_frame.saturating_mul(channels);
        let mixed = (0..channels)
            .map(|channel| {
                window
                    .samples
                    .get(start + channel)
                    .copied()
                    .unwrap_or_default()
            })
            .sum::<f32>()
            / channels as f32;
        out.push((mixed.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16);
    }
    out
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn audio_seconds(window: &AudioWindow) -> f32 {
    window.frames as f32 / window.format.sample_rate_hz.max(1) as f32
}
