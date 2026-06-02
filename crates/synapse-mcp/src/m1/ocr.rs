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
                text: word.text,
                bbox: word.bbox,
                confidence: normalize_confidence(word.confidence),
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
        .map(|word| normalize_confidence(word.confidence))
        .sum::<f32>();
    let count = u16::try_from(regions.len()).unwrap_or(u16::MAX);
    sum / f32::from(count)
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
