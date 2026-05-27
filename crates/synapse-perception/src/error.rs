use synapse_core::{Rect, error_codes};
use thiserror::Error;

pub type PerceptionResult<T> = Result<T, PerceptionError>;

#[derive(Debug, Error)]
pub enum PerceptionError {
    #[error("OCR produced no text for region {region:?}")]
    OcrNoText { region: Rect },
    #[error("OCR backend is unavailable: {detail}")]
    OcrBackendUnavailable { detail: String },
    #[error("no perception source is available: {detail}")]
    ObserveNoPerceptionAvailable { detail: String },
    #[error("observe failed internally: {detail}")]
    ObserveInternal { detail: String },
    #[error("HUD extraction failed: {detail}")]
    HudExtractionFailed { detail: String },
    #[error("invalid perception mode: {value}")]
    PerceptionModeInvalid { value: String },
}

impl PerceptionError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::OcrNoText { .. } => error_codes::OCR_NO_TEXT,
            Self::OcrBackendUnavailable { .. } => error_codes::OCR_BACKEND_UNAVAILABLE,
            Self::ObserveNoPerceptionAvailable { .. } => {
                error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE
            }
            Self::ObserveInternal { .. } => error_codes::OBSERVE_INTERNAL,
            Self::HudExtractionFailed { .. } => error_codes::HUD_EXTRACTION_FAILED,
            Self::PerceptionModeInvalid { .. } => error_codes::PERCEPTION_MODE_INVALID,
        }
    }
}
