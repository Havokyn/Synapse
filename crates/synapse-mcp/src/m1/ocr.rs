use rmcp::ErrorData;
use synapse_core::{OcrBackend, OcrResult, OcrWord, Rect, error_codes};
use synapse_perception::{TextRegion, read_text as platform_read_text, read_text_with_provider};

use crate::m1::{M1State, ReadTextParams, current_input, mcp_error};

#[derive(Clone, Debug)]
pub struct ResolvedReadTextRequest {
    pub region: Rect,
    pub requested_backend: OcrBackend,
    pub effective_backend: OcrBackend,
    pub lang_hint: Option<String>,
    pub synthetic: bool,
}

impl ResolvedReadTextRequest {
    #[must_use]
    pub fn lang(&self) -> String {
        self.lang_hint
            .as_deref()
            .map(str::trim)
            .filter(|lang| !lang.is_empty())
            .unwrap_or("und")
            .to_owned()
    }
}

pub fn resolve_read_text_request(
    state: &M1State,
    params: &ReadTextParams,
) -> Result<ResolvedReadTextRequest, ErrorData> {
    let region = text_region(state, params)?;
    validate_ocr_region(region)?;
    Ok(ResolvedReadTextRequest {
        region,
        requested_backend: params.backend,
        effective_backend: effective_ocr_backend(params.backend)?,
        lang_hint: params.lang_hint.clone(),
        synthetic: state.synthetic.is_some(),
    })
}

pub fn read_text_request_uncached(
    request: &ResolvedReadTextRequest,
) -> Result<OcrResult, ErrorData> {
    if request.synthetic {
        let provider = SyntheticOcrProvider {
            region: request.region,
        };
        let words = read_text_with_provider(&provider, request.region)
            .map_err(|err| mcp_error(err.code(), err.to_string()))?;
        return Ok(ocr_result_from_text_regions(words, request));
    }
    match request.effective_backend {
        OcrBackend::Winrt => {
            let words = platform_read_text(request.region)
                .map_err(|err| mcp_error(err.code(), err.to_string()))?;
            Ok(ocr_result_from_text_regions(words, request))
        }
        OcrBackend::Crnn => Err(crnn_unavailable_error()),
        OcrBackend::Auto => Err(mcp_error(
            error_codes::OCR_BACKEND_UNAVAILABLE,
            "internal OCR backend resolution left backend=auto after request validation",
        )),
    }
}

#[cfg(windows)]
pub fn read_text_request_from_bgra(
    request: &ResolvedReadTextRequest,
    captured: &synapse_capture::CapturedBgraBitmap,
) -> Result<OcrResult, ErrorData> {
    if request.synthetic {
        return read_text_request_uncached(request);
    }
    match request.effective_backend {
        OcrBackend::Winrt => {
            let words = synapse_perception::read_text_from_bgra_bitmap(
                request.region,
                captured.width,
                captured.height,
                &captured.bytes,
            )
            .map_err(|err| mcp_error(err.code(), err.to_string()))?;
            Ok(ocr_result_from_text_regions(words, request))
        }
        OcrBackend::Crnn => Err(crnn_unavailable_error()),
        OcrBackend::Auto => Err(mcp_error(
            error_codes::OCR_BACKEND_UNAVAILABLE,
            "internal OCR backend resolution left backend=auto after request validation",
        )),
    }
}

/// Runs WinRT OCR over a web element's captured BGRA bitmap and returns an
/// `OcrResult` whose word boxes are relative to the captured element (#703).
///
/// Used by the `read_text` handler when `element_id` is a CDP/web node, which
/// the UIA element-bounds path cannot resolve. The bitmap comes from a CDP
/// element-clipped screenshot, so the OCR region is the whole bitmap.
///
/// # Errors
///
/// `OCR_NO_TEXT` if the bitmap dimensions exceed `i32` or OCR finds no text;
/// any `WinRT` OCR backend error from `read_text_from_bgra_bitmap`.
#[cfg(windows)]
pub fn ocr_result_from_web_bitmap(
    width: u32,
    height: u32,
    bgra: &[u8],
    lang_hint: Option<&str>,
) -> Result<OcrResult, ErrorData> {
    let w = i32::try_from(width).map_err(|_| {
        mcp_error(
            error_codes::OCR_NO_TEXT,
            format!("web element OCR bitmap width {width} exceeds i32"),
        )
    })?;
    let h = i32::try_from(height).map_err(|_| {
        mcp_error(
            error_codes::OCR_NO_TEXT,
            format!("web element OCR bitmap height {height} exceeds i32"),
        )
    })?;
    let region = Rect { x: 0, y: 0, w, h };
    let words = synapse_perception::read_text_from_bgra_bitmap(region, width, height, bgra)
        .map_err(|err| mcp_error(err.code(), err.to_string()))?;
    let request = ResolvedReadTextRequest {
        region,
        requested_backend: OcrBackend::Auto,
        effective_backend: OcrBackend::Winrt,
        lang_hint: lang_hint.map(str::to_owned),
        synthetic: false,
    };
    Ok(ocr_result_from_text_regions(words, &request))
}

fn text_region(state: &M1State, params: &ReadTextParams) -> Result<Rect, ErrorData> {
    if let Some(region) = params.region {
        return Ok(region);
    }
    if let Some(element_id) = &params.element_id {
        if state.synthetic.is_none() {
            return synapse_a11y::element_bounding_rect(element_id).map_err(|err| {
                mcp_error(
                    error_codes::OCR_NO_TEXT,
                    format!("element_id has no live visible OCR region: {err}"),
                )
            });
        }
        let input = current_input(state, 2)?;
        return input
            .elements
            .iter()
            .find(|node| &node.element_id == element_id)
            .map(|node| node.bbox)
            .ok_or_else(|| {
                mcp_error(
                    error_codes::OCR_NO_TEXT,
                    "element_id has no visible OCR region",
                )
            });
    }

    let input = current_input(state, 2)?;
    input.focused.map(|focused| focused.bbox).ok_or_else(|| {
        mcp_error(
            error_codes::OCR_NO_TEXT,
            "read_text requires region, element_id, or a focused element with a visible OCR region",
        )
    })
}

fn validate_ocr_region(region: Rect) -> Result<(), ErrorData> {
    if region.w <= 0 || region.h <= 0 {
        return Err(mcp_error(
            error_codes::OCR_NO_TEXT,
            format!(
                "read_text OCR region must be non-empty: bbox=({}, {}, {}, {})",
                region.x, region.y, region.w, region.h
            ),
        ));
    }
    Ok(())
}

fn effective_ocr_backend(backend: OcrBackend) -> Result<OcrBackend, ErrorData> {
    match backend {
        OcrBackend::Winrt | OcrBackend::Auto => Ok(OcrBackend::Winrt),
        OcrBackend::Crnn => Err(crnn_unavailable_error()),
    }
}

fn crnn_unavailable_error() -> ErrorData {
    mcp_error(
        error_codes::OCR_BACKEND_UNAVAILABLE,
        "CRNN OCR backend is declared in schema but no CRNN runtime/model provider is wired on this host; request backend=winrt or backend=auto",
    )
}

fn ocr_result_from_text_regions(
    regions: Vec<TextRegion>,
    request: &ResolvedReadTextRequest,
) -> OcrResult {
    let full_text = regions
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let confidence = aggregate_confidence(&regions);
    OcrResult {
        full_text,
        words: regions
            .into_iter()
            .map(|word| OcrWord {
                confidence: identifier_aware_confidence(&word.text, word.confidence),
                text: word.text,
                bbox: word.bbox,
            })
            .collect(),
        confidence,
        region: request.region,
        lang: request.lang(),
    }
}

fn aggregate_confidence(regions: &[TextRegion]) -> f32 {
    if regions.is_empty() {
        return 0.0;
    }
    let sum = regions
        .iter()
        .map(|word| identifier_aware_confidence(&word.text, word.confidence))
        .sum::<f32>();
    let count = u16::try_from(regions.len()).unwrap_or(u16::MAX);
    sum / f32::from(count)
}

fn identifier_aware_confidence(text: &str, confidence: f32) -> f32 {
    let confidence = normalize_confidence(confidence);
    if is_ambiguous_identifier_token(text) {
        confidence.min(AMBIGUOUS_IDENTIFIER_CONFIDENCE_CAP)
    } else {
        confidence
    }
}

const AMBIGUOUS_IDENTIFIER_CONFIDENCE_CAP: f32 = 0.74;

fn is_ambiguous_identifier_token(text: &str) -> bool {
    let token = text.trim_matches(|ch: char| !identifier_char(ch));
    if token.len() < 2 || !token.chars().any(ambiguous_identifier_char) {
        return false;
    }
    let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
    let has_separator = token.chars().any(identifier_separator);
    let lower = token.to_ascii_lowercase();
    let has_ambiguous_pair = [
        "v1", "vl", "vi", "i1", "l1", "1l", "1i", "o0", "0o", "ol", "lo",
    ]
    .iter()
    .any(|pair| lower.contains(pair));
    has_digit || has_separator || has_ambiguous_pair || lower == "vl" || lower == "v1"
}

const fn identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '-' | '_' | '.' | ':' | '/' | '\\' | '|' | '[' | ']' | '(' | ')'
        )
}

const fn identifier_separator(ch: char) -> bool {
    matches!(ch, '-' | '_' | '.' | ':' | '/' | '\\' | '|')
}

const fn ambiguous_identifier_char(ch: char) -> bool {
    matches!(ch, '0' | 'O' | 'o' | '1' | 'I' | 'l' | '|' | 'v' | 'V')
}

const fn normalize_confidence(confidence: f32) -> f32 {
    if confidence.is_finite() {
        confidence.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

struct SyntheticOcrProvider {
    region: Rect,
}

impl synapse_perception::OcrProvider for SyntheticOcrProvider {
    fn read_text(&self, _region: Rect) -> synapse_perception::PerceptionResult<Vec<TextRegion>> {
        Ok(vec![TextRegion {
            text: "Synapse".to_owned(),
            bbox: Rect {
                x: self.region.x.saturating_add(4),
                y: self.region.y.saturating_add(4),
                w: 72,
                h: 18,
            },
            confidence: 0.99,
        }])
    }
}
