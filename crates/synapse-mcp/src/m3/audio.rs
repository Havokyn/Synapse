use rmcp::{ErrorData, schemars::JsonSchema};
use schemars::{Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use synapse_audio::{
    AudioRuntime, AudioWindow, MAX_RING_SECONDS, Transcription,
    ring::{DEFAULT_SAMPLE_RATE_HZ, STEREO_CHANNELS},
};
use synapse_core::error_codes;

use crate::{
    m1::mcp_error,
    m3::{M3ToolStub, SharedM3State},
};

const DEFAULT_SECONDS: u32 = 5;
const DEFAULT_LANGUAGE: &str = "en";
const PCM_FORMAT: &str = "s16le";
const WHISPER_TINY_MODEL_ID: &str = "whisper_tiny_int8";
const BYTES_PER_SAMPLE: usize = 2;

const fn default_seconds() -> u32 {
    DEFAULT_SECONDS
}

fn default_language() -> String {
    DEFAULT_LANGUAGE.to_owned()
}

fn seconds_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "format": "uint32",
        "minimum": 0,
        "maximum": MAX_RING_SECONDS,
        "default": DEFAULT_SECONDS
    })
}

fn language_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "default": DEFAULT_LANGUAGE
    })
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioTailParams {
    #[serde(default = "default_seconds")]
    #[schemars(schema_with = "seconds_schema")]
    pub seconds: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioTailResponse {
    pub pcm: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub format: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioTranscribeParams {
    #[serde(default = "default_seconds")]
    #[schemars(schema_with = "seconds_schema")]
    pub seconds: u32,
    #[serde(default = "default_language")]
    #[schemars(schema_with = "language_schema")]
    pub language: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioTranscribeResponse {
    pub text: String,
    pub confidence: f32,
    pub latency_ms: u64,
    pub model_id: String,
}

#[must_use]
pub const fn audio_tail() -> M3ToolStub {
    M3ToolStub::new("audio_tail")
}

#[must_use]
pub const fn audio_transcribe() -> M3ToolStub {
    M3ToolStub::new("audio_transcribe")
}

pub fn tail_audio(
    m3_state: &SharedM3State,
    params: &AudioTailParams,
) -> Result<AudioTailResponse, ErrorData> {
    validate_seconds(params.seconds)?;
    if params.seconds == 0 {
        return Ok(AudioTailResponse {
            pcm: Vec::new(),
            sample_rate: DEFAULT_SAMPLE_RATE_HZ,
            channels: STEREO_CHANNELS,
            format: PCM_FORMAT.to_owned(),
        });
    }

    let runtime = m3_state
        .lock()
        .map_err(|_err| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "M3 service state lock poisoned",
            )
        })?
        .ensure_audio_runtime()
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    tail_audio_from_runtime(&runtime, params.seconds)
}

pub fn tail_audio_from_runtime(
    runtime: &AudioRuntime,
    seconds: u32,
) -> Result<AudioTailResponse, ErrorData> {
    validate_seconds(seconds)?;
    let seconds_f32 = seconds_to_f32(seconds)?;
    let window = runtime
        .tail_seconds(seconds_f32)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    Ok(response_from_window(&window, seconds))
}

pub fn transcribe_audio(
    m3_state: &SharedM3State,
    params: &AudioTranscribeParams,
) -> Result<AudioTranscribeResponse, ErrorData> {
    validate_seconds(params.seconds)?;
    let language = normalize_language_param(&params.language)?;
    let runtime = m3_state
        .lock()
        .map_err(|_err| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "M3 service state lock poisoned",
            )
        })?
        .ensure_audio_runtime()
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    transcribe_audio_from_runtime(&runtime, params.seconds, language)
}

pub fn transcribe_audio_from_runtime(
    runtime: &AudioRuntime,
    seconds: u32,
    language: &str,
) -> Result<AudioTranscribeResponse, ErrorData> {
    validate_seconds(seconds)?;
    let language = normalize_language_param(language)?;
    let seconds_f32 = seconds_to_f32(seconds)?;
    let transcription = runtime
        .transcribe_tail(seconds_f32, language)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    Ok(response_from_transcription(transcription))
}

fn response_from_window(window: &AudioWindow, seconds: u32) -> AudioTailResponse {
    let requested_samples = requested_samples(window, seconds);
    let mut pcm = Vec::with_capacity(requested_samples.saturating_mul(BYTES_PER_SAMPLE));
    let missing_samples = requested_samples.saturating_sub(window.samples.len());
    pcm.resize(missing_samples.saturating_mul(BYTES_PER_SAMPLE), 0);
    pcm.extend_from_slice(&window.pcm_i16_le());

    AudioTailResponse {
        pcm,
        sample_rate: window.format.sample_rate_hz,
        channels: window.format.channels,
        format: PCM_FORMAT.to_owned(),
    }
}

fn requested_samples(window: &AudioWindow, seconds: u32) -> usize {
    usize::try_from(window.format.sample_rate_hz)
        .unwrap_or(usize::MAX)
        .saturating_mul(seconds as usize)
        .saturating_mul(usize::from(window.format.channels))
}

fn validate_seconds(seconds: u32) -> Result<(), ErrorData> {
    if seconds > MAX_RING_SECONDS {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("audio_tail seconds must be <= {MAX_RING_SECONDS}; got {seconds}"),
        ));
    }
    Ok(())
}

fn normalize_language_param(language: &str) -> Result<&'static str, ErrorData> {
    let language = language.trim();
    if language.is_empty() || language.eq_ignore_ascii_case(DEFAULT_LANGUAGE) {
        Ok(DEFAULT_LANGUAGE)
    } else {
        Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("audio_transcribe language must be {DEFAULT_LANGUAGE:?}; got {language:?}"),
        ))
    }
}

fn seconds_to_f32(seconds: u32) -> Result<f32, ErrorData> {
    u16::try_from(seconds).map(f32::from).map_err(|_error| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("audio_tail seconds must fit u16; got {seconds}"),
        )
    })
}

fn response_from_transcription(transcription: Transcription) -> AudioTranscribeResponse {
    AudioTranscribeResponse {
        text: transcription.text,
        confidence: transcription.confidence,
        latency_ms: u64::try_from(transcription.elapsed_ms).unwrap_or(u64::MAX),
        model_id: WHISPER_TINY_MODEL_ID.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use synapse_audio::{AudioConfig, AudioFormat, AudioRuntime};

    use super::*;

    #[test]
    fn response_pads_partial_ring_to_requested_byte_count() -> anyhow::Result<()> {
        let runtime = AudioRuntime::spawn(AudioConfig::default())?;
        let ring = runtime.ring();
        ring.set_format(AudioFormat {
            sample_rate_hz: 48_000,
            channels: 2,
        });
        ring.push_interleaved(&vec![0.25; 48_000 * 2]);

        let response = tail_audio_from_runtime(&runtime, 2)
            .map_err(|error| anyhow::anyhow!("tail_audio failed: {error:?}"))?;

        assert_eq!(response.sample_rate, 48_000);
        assert_eq!(response.channels, 2);
        assert_eq!(response.format, PCM_FORMAT);
        assert_eq!(response.pcm.len(), 2 * 48_000 * 2 * BYTES_PER_SAMPLE);
        assert!(
            response.pcm[..48_000 * 2 * BYTES_PER_SAMPLE]
                .iter()
                .all(|byte| *byte == 0)
        );
        assert!(
            response.pcm[48_000 * 2 * BYTES_PER_SAMPLE..]
                .iter()
                .any(|byte| *byte != 0)
        );
        Ok(())
    }

    #[test]
    fn transcribe_maps_silence_without_model_load_and_rejects_language() -> anyhow::Result<()> {
        let runtime = AudioRuntime::spawn(AudioConfig::default())?;

        let blank = transcribe_audio_from_runtime(&runtime, 5, "en")
            .map_err(|error| anyhow::anyhow!("transcribe silence failed: {error:?}"))?;
        assert_eq!(blank.text, "");
        assert_eq!(blank.confidence, 0.0);
        assert_eq!(blank.latency_ms, 0);
        assert_eq!(blank.model_id, WHISPER_TINY_MODEL_ID);

        let invalid = transcribe_audio_from_runtime(&runtime, 5, "xx")
            .expect_err("unsupported language should fail before STT");
        assert_eq!(
            error_data_code(&invalid),
            Some(error_codes::TOOL_PARAMS_INVALID)
        );
        Ok(())
    }

    #[test]
    fn transcribe_non_silence_maps_missing_model_code() -> anyhow::Result<()> {
        let runtime = AudioRuntime::spawn(AudioConfig {
            stt_model_path: Some("missing-whisper-tiny-int8.onnx".into()),
            ..AudioConfig::default()
        })?;
        let ring = runtime.ring();
        ring.set_format(AudioFormat {
            sample_rate_hz: 16_000,
            channels: 1,
        });
        ring.push_interleaved(&vec![0.5; 16_000]);

        let error = transcribe_audio_from_runtime(&runtime, 1, "en")
            .expect_err("missing model should fail for non-silent audio");
        assert_eq!(
            error_data_code(&error),
            Some(error_codes::AUDIO_STT_MODEL_NOT_LOADED)
        );
        Ok(())
    }

    fn error_data_code(error: &ErrorData) -> Option<&str> {
        error.data.as_ref()?.get("code")?.as_str()
    }
}
