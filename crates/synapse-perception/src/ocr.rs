use serde::{Deserialize, Serialize};
use synapse_core::Rect;

use crate::{PerceptionError, PerceptionResult};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextRegion {
    pub text: String,
    pub bbox: Rect,
    pub confidence: f32,
}

pub trait OcrProvider {
    /// Reads text from a screen-coordinate region.
    ///
    /// # Errors
    ///
    /// Returns a structured perception error when OCR cannot run or finds no text.
    fn read_text(&self, region: Rect) -> PerceptionResult<Vec<TextRegion>>;
}

/// Reads OCR text from a screen-coordinate region.
///
/// # Errors
///
/// Returns `OCR_NO_TEXT` for an empty region and `OCR_BACKEND_UNAVAILABLE`
/// when the platform OCR backend cannot run.
pub fn read_text(region: Rect) -> PerceptionResult<Vec<TextRegion>> {
    if is_empty_region(region) {
        return Err(PerceptionError::OcrNoText { region });
    }
    platform::read_text(region)
}

/// Reads OCR text with an injected provider.
///
/// # Errors
///
/// Returns `OCR_NO_TEXT` for invalid/empty regions or empty provider output.
pub fn read_text_with_provider(
    provider: &dyn OcrProvider,
    region: Rect,
) -> PerceptionResult<Vec<TextRegion>> {
    if is_empty_region(region) {
        return Err(PerceptionError::OcrNoText { region });
    }
    let words = provider.read_text(region)?;
    if words.is_empty() {
        return Err(PerceptionError::OcrNoText { region });
    }
    Ok(words)
}

#[must_use]
pub const fn is_empty_region(region: Rect) -> bool {
    region.w <= 0 || region.h <= 0
}

#[cfg(windows)]
/// Runs `WinRT` OCR over a caller-provided `SoftwareBitmap`.
///
/// # Errors
///
/// Returns `OCR_BACKEND_UNAVAILABLE` when `WinRT` cannot initialize or rejects
/// the bitmap, and `OCR_NO_TEXT` when OCR completes with no recognized words.
pub fn read_text_from_software_bitmap(
    region: Rect,
    bitmap: &windows::Graphics::Imaging::SoftwareBitmap,
) -> PerceptionResult<Vec<TextRegion>> {
    if is_empty_region(region) {
        return Err(PerceptionError::OcrNoText { region });
    }
    platform::read_text_from_software_bitmap(region, bitmap)
}

#[cfg(not(windows))]
mod platform {
    use synapse_core::Rect;

    use super::{PerceptionError, PerceptionResult, TextRegion};

    pub fn read_text(_region: Rect) -> PerceptionResult<Vec<TextRegion>> {
        Err(PerceptionError::OcrBackendUnavailable {
            detail: "Windows.Media.Ocr requires Windows".to_owned(),
        })
    }
}

#[cfg(windows)]
mod platform {
    use std::sync::OnceLock;

    use synapse_capture::screen_region_to_software_bitmap;
    use synapse_core::Rect;
    use windows::Media::Ocr::{OcrEngine, OcrResult};

    use super::{PerceptionError, PerceptionResult, TextRegion};

    pub fn read_text(region: Rect) -> PerceptionResult<Vec<TextRegion>> {
        let captured = capture_region_bitmap(region)?;
        read_text_from_software_bitmap(captured.region, &captured.bitmap)
    }

    pub fn read_text_from_software_bitmap(
        region: Rect,
        bitmap: &windows::Graphics::Imaging::SoftwareBitmap,
    ) -> PerceptionResult<Vec<TextRegion>> {
        let engine = ocr_engine()?;
        let result = engine
            .RecognizeAsync(bitmap)
            .map_err(|err| backend_unavailable(err.to_string()))?
            .join()
            .map_err(|err| backend_unavailable(err.to_string()))?;
        text_regions_from_result(region, &result)
    }

    fn ocr_engine() -> PerceptionResult<&'static OcrEngine> {
        static ENGINE: OnceLock<Result<OcrEngine, String>> = OnceLock::new();
        ENGINE
            .get_or_init(|| {
                OcrEngine::TryCreateFromUserProfileLanguages().map_err(|err| err.to_string())
            })
            .as_ref()
            .map_err(|detail| backend_unavailable(detail.clone()))
    }

    fn text_regions_from_result(
        region: Rect,
        result: &OcrResult,
    ) -> PerceptionResult<Vec<TextRegion>> {
        let lines = result
            .Lines()
            .map_err(|err| backend_unavailable(err.to_string()))?;
        let mut output = Vec::new();
        for line_index in 0..lines
            .Size()
            .map_err(|err| backend_unavailable(err.to_string()))?
        {
            let line = lines
                .GetAt(line_index)
                .map_err(|err| backend_unavailable(err.to_string()))?;
            let words = line
                .Words()
                .map_err(|err| backend_unavailable(err.to_string()))?;
            for word_index in 0..words
                .Size()
                .map_err(|err| backend_unavailable(err.to_string()))?
            {
                let word = words
                    .GetAt(word_index)
                    .map_err(|err| backend_unavailable(err.to_string()))?;
                let bbox = word
                    .BoundingRect()
                    .map_err(|err| backend_unavailable(err.to_string()))?;
                output.push(TextRegion {
                    text: word
                        .Text()
                        .map_err(|err| backend_unavailable(err.to_string()))?
                        .to_string_lossy(),
                    bbox: Rect {
                        x: region.x.saturating_add(round_to_i32(bbox.X)),
                        y: region.y.saturating_add(round_to_i32(bbox.Y)),
                        w: round_to_i32(bbox.Width),
                        h: round_to_i32(bbox.Height),
                    },
                    confidence: 1.0,
                });
            }
        }
        if output.is_empty() {
            Err(PerceptionError::OcrNoText { region })
        } else {
            Ok(output)
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn round_to_i32(value: f32) -> i32 {
        let value = f64::from(value);
        if !value.is_finite() {
            0
        } else if value >= f64::from(i32::MAX) {
            i32::MAX
        } else if value <= f64::from(i32::MIN) {
            i32::MIN
        } else {
            value.round() as i32
        }
    }

    const fn backend_unavailable(detail: String) -> PerceptionError {
        PerceptionError::OcrBackendUnavailable { detail }
    }

    fn capture_region_bitmap(
        region: Rect,
    ) -> PerceptionResult<synapse_capture::CapturedSoftwareBitmap> {
        screen_region_to_software_bitmap(region).map_err(|err| backend_unavailable(err.to_string()))
    }
}
