use super::{
    ErrorData, FindParams, FindResponse, Health, Json, ObserveParams, Parameters, ReadTextParams,
    SetCaptureTargetParams, SetCaptureTargetResponse, SetPerceptionModeParams,
    SetPerceptionModeResponse, SynapseService, empty_input_schema, mcp_error, observe_include,
    observe_input, populate_audio_summary, populate_clipboard_summary,
    populate_detection_from_state, populate_fs_recent, read_text_request_uncached,
    resolve_read_text_request, set_capture_target_in_state, set_perception_mode_in_state, tool,
    tool_router,
};

#[cfg(windows)]
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

#[cfg(windows)]
use chrono::{DateTime, Utc};
#[cfg(windows)]
use image::{GrayImage, Luma};
#[cfg(windows)]
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use sha2::{Digest as _, Sha256};
use synapse_core::{HudFieldError, HudReadings, OcrResult, Profile, error_codes};
use synapse_perception::ObservationAssembler;
#[cfg(windows)]
use synapse_storage::{cf, decode_json, encode_json};

#[cfg(windows)]
use synapse_core::{
    HudExtractor, HudFieldSpec, HudReading, OcrBackend, Point, Rect, SCHEMA_VERSION,
};
#[cfg(windows)]
use synapse_perception::{
    FieldExtractionRequest, HudTemplate, OcrProvider, PerceptionError, PerceptionResult,
    SystemOcrProvider, TextRegion, extract_field, parse_hud_text, resolve_hud_region_rect,
};
#[cfg(windows)]
use synapse_reflex::ReflexRuntime;

#[tool_router(router = m1_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(description = "Return server health", input_schema = empty_input_schema())]
    pub async fn health(&self) -> Json<Health> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "health",
            "tool.invocation kind=health"
        );
        Json(self.health_payload())
    }

    #[tool(description = "Returns structured state of the focused window and surrounding context")]
    pub async fn observe(
        &self,
        params: Parameters<ObserveParams>,
    ) -> Result<Json<synapse_core::Observation>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "observe",
            "tool.invocation kind=observe"
        );
        let include = observe_include(&params.0);
        // Scope the (non-Send) state guard so it is released before any await.
        let mut input = {
            let state = self.m1_state()?;
            let mut input = observe_input(&state, &params.0)?;
            if include.fs && input.fs_recent.is_empty() {
                populate_fs_recent(&mut input, &state.fs_recent_tracker);
            }
            input
        };
        if let Some(since) = params.0.since_event_seq {
            input.recent_events.retain(|event| event.seq > since);
        }

        if include.elements {
            super::enrich_input_with_cdp(
                &mut input,
                include.max_subtree_depth,
                include.max_subtree_nodes,
            )
            .await;
            super::enrich_input_with_browser_ocr(&mut input, include.max_subtree_nodes);
        }

        if include.audio && input.audio == synapse_core::AudioContext::default() {
            populate_audio_summary(&self.m3_state, &mut input);
        }
        if include.clipboard && input.clipboard_summary.is_none() {
            populate_clipboard_summary(&mut input);
        }
        self.resolve_input_profile_and_hud(&mut input, include.hud);
        if include.events {
            self.populate_everquest_log_events(&mut input);
        }
        {
            let mut state = self.m1_state()?;
            populate_detection_from_state(&mut state, &mut input);
        }
        let observation = ObservationAssembler::new()
            .assemble(include, input)
            .map_err(|err| mcp_error(err.code(), err.to_string()))?;

        let mut state = self.m1_state()?;
        state.last_observed_foreground = Some(observation.foreground.clone());
        drop(state);
        self.persist_observation(&observation, "observe")?;
        Ok(Json(observation))
    }

    #[tool(description = "Search visible accessibility nodes and detected entities")]
    pub async fn find(
        &self,
        params: Parameters<FindParams>,
    ) -> Result<Json<FindResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "find",
            "tool.invocation kind=find"
        );
        let mut input = {
            let mut state = self.m1_state()?;
            super::build_find_input(&mut state, &params.0)?
        };
        super::enrich_input_with_cdp(
            &mut input,
            super::find_snapshot_depth(),
            super::find_cdp_max_nodes(),
        )
        .await;
        super::enrich_input_with_browser_ocr(&mut input, super::find_cdp_max_nodes());
        Ok(Json(super::match_find_input(&input, &params.0)))
    }

    #[tool(description = "OCR text from a screen region or visible element")]
    pub async fn read_text(
        &self,
        params: Parameters<ReadTextParams>,
    ) -> Result<Json<synapse_core::OcrResult>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "read_text",
            "tool.invocation kind=read_text"
        );
        let request = {
            let state = self.m1_state()?;
            resolve_read_text_request(&state, &params.0)?
        };
        self.read_text_request_with_cache(request).map(Json)
    }

    #[tool(description = "Set the active capture target")]
    pub async fn set_capture_target(
        &self,
        params: Parameters<SetCaptureTargetParams>,
    ) -> Result<Json<SetCaptureTargetResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "set_capture_target",
            "tool.invocation kind=set_capture_target"
        );
        let mut state = self.m1_state()?;
        set_capture_target_in_state(&mut state, params.0).map(Json)
    }

    #[tool(description = "Set the active perception mode")]
    pub async fn set_perception_mode(
        &self,
        params: Parameters<SetPerceptionModeParams>,
    ) -> Result<Json<SetPerceptionModeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "set_perception_mode",
            "tool.invocation kind=set_perception_mode"
        );
        let mut state = self.m1_state()?;
        set_perception_mode_in_state(&mut state, &params.0).map(Json)
    }
}

impl SynapseService {
    pub(super) fn resolve_input_profile_and_hud(
        &self,
        input: &mut synapse_perception::ObservationInput,
        include_hud: bool,
    ) {
        match self.reevaluate_profile_for_foreground(&input.foreground) {
            Ok(transition) => {
                let Some(profile_id) = transition.active_profile_id.clone() else {
                    tracing::debug!(
                        code = "PROFILE_FOREGROUND_UNMATCHED",
                        "observed foreground did not match a loaded profile"
                    );
                    return;
                };
                tracing::info!(
                    code = "PROFILE_FOREGROUND_MATCHED",
                    profile_id = %profile_id,
                    rank = ?transition.resolution.as_ref().map(|resolution| resolution.rank_name),
                    "observed foreground matched profile"
                );
                input.foreground.profile_id = Some(profile_id.clone());
                let Ok(runtime) = self.profile_runtime() else {
                    tracing::warn!(
                        code = "PROFILE_FOREGROUND_RESOLUTION_SKIPPED",
                        "profile runtime unavailable while resolving observed foreground profile config"
                    );
                    return;
                };
                match runtime.profile(&profile_id) {
                    Ok(Some(profile)) => {
                        if let Err(error) = self.apply_m1_runtime_config_for_profile(&profile) {
                            tracing::warn!(
                                code = "PROFILE_M1_RUNTIME_CONFIG_FAILED",
                                profile_id = %profile_id,
                                error = %error,
                                "profile runtime config failed for observed foreground"
                            );
                        } else if let Ok(state) = self.m1_state() {
                            input.mode_override = Some(state.perception_mode);
                            input.capture_config = Some(state.active_capture_config.clone());
                            input.capture_runtime = Some(state.capture_runtime_readback());
                        }
                        if include_hud {
                            populate_profile_hud(input, &profile, runtime.profile_dir());
                        }
                    }
                    Ok(None) => {
                        tracing::warn!(
                            code = "PROFILE_HUD_PROFILE_MISSING",
                            profile_id = %profile_id,
                            "profile resolved but could not be loaded for HUD extraction"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            code = "PROFILE_HUD_PROFILE_LOAD_FAILED",
                            profile_id = %profile_id,
                            error = %error,
                            "profile load failed for HUD extraction"
                        );
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    code = "PROFILE_FOREGROUND_RESOLUTION_FAILED",
                    error = %error,
                    "profile resolver failed for observed foreground"
                );
            }
        }
    }
}

impl SynapseService {
    #[cfg(windows)]
    fn read_text_request_with_cache(
        &self,
        request: crate::m1::ResolvedReadTextRequest,
    ) -> Result<OcrResult, ErrorData> {
        if request.synthetic || request.effective_backend != OcrBackend::Winrt {
            return read_text_request_uncached(&request);
        }

        let captured =
            synapse_capture::screen_region_to_bgra_bitmap(request.region).map_err(|error| {
                mcp_error(
                    error.code(),
                    format!(
                        "OCR screen capture failed for region {:?}: {error}",
                        request.region
                    ),
                )
            })?;
        let bitmap_sha256 = sha256_hex(&captured.bytes);
        let cache_key = ocr_cache_key(&request, captured.width, captured.height, &bitmap_sha256);
        let runtime = self.reflex_runtime()?;

        {
            let runtime = lock_reflex_runtime(&runtime)?;
            if let Some(row) = read_ocr_cache_row(
                &runtime,
                &cache_key,
                &request,
                captured.width,
                captured.height,
                &bitmap_sha256,
            )? {
                tracing::info!(
                    code = "OCR_CACHE_HIT",
                    cache_key = %cache_key,
                    backend = ocr_backend_name(request.effective_backend),
                    region_x = request.region.x,
                    region_y = request.region.y,
                    region_w = request.region.w,
                    region_h = request.region.h,
                    word_count = row.word_count,
                    recognition_latency_ms = row.recognition_latency_ms,
                    "OCR cache hit"
                );
                return Ok(row.result);
            }
        }

        let recognition_start = Instant::now();
        let result = crate::m1::read_text_request_from_bgra(&request, &captured)?;
        let recognition_latency_ms = elapsed_ms_u64(recognition_start);
        let row = OcrCacheRow {
            schema_version: SCHEMA_VERSION,
            cache_key: cache_key.clone(),
            created_at: Utc::now(),
            requested_backend: request.requested_backend,
            effective_backend: request.effective_backend,
            lang: request.lang(),
            region: request.region,
            bitmap_sha256: bitmap_sha256.clone(),
            bitmap_width: captured.width,
            bitmap_height: captured.height,
            bitmap_bytes: captured.bytes.len() as u64,
            result: result.clone(),
            recognition_latency_ms,
            word_count: result.words.len() as u64,
        };
        let encoded = encode_json(&row).map_err(|error| {
            mcp_error(
                error.code(),
                format!("OCR cache row encode failed for key {cache_key}: {error}"),
            )
        })?;
        {
            let runtime = lock_reflex_runtime(&runtime)?;
            if !runtime.storage_pressure_permits_write(cf::CF_OCR_CACHE) {
                return Err(mcp_error(
                    error_codes::STORAGE_WRITE_FAILED,
                    format!(
                        "OCR cache write refused under disk pressure: cf_name={} key={cache_key}",
                        cf::CF_OCR_CACHE
                    ),
                ));
            }
            runtime
                .storage_put_rows(
                    cf::CF_OCR_CACHE,
                    vec![(cache_key.as_bytes().to_vec(), encoded)],
                )
                .map_err(|error| {
                    mcp_error(
                        error.code(),
                        format!("OCR cache write failed for key {cache_key}: {error}"),
                    )
                })?;
            let readback = read_ocr_cache_row(
                &runtime,
                &cache_key,
                &request,
                captured.width,
                captured.height,
                &bitmap_sha256,
            )?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::STORAGE_WRITE_FAILED,
                    format!("OCR cache write had no readback row: key={cache_key}"),
                )
            })?;
            if readback.result != result {
                return Err(mcp_error(
                    error_codes::STORAGE_WRITE_FAILED,
                    format!("OCR cache readback result mismatch for key {cache_key}"),
                ));
            }
        }

        tracing::info!(
            code = "OCR_CACHE_MISS_RECORDED",
            cache_key = %cache_key,
            backend = ocr_backend_name(request.effective_backend),
            region_x = request.region.x,
            region_y = request.region.y,
            region_w = request.region.w,
            region_h = request.region.h,
            word_count = result.words.len(),
            recognition_latency_ms,
            "OCR cache miss recorded"
        );
        Ok(result)
    }

    #[cfg(not(windows))]
    fn read_text_request_with_cache(
        &self,
        request: crate::m1::ResolvedReadTextRequest,
    ) -> Result<OcrResult, ErrorData> {
        read_text_request_uncached(&request)
    }
}

#[cfg(windows)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OcrCacheRow {
    schema_version: u32,
    cache_key: String,
    created_at: DateTime<Utc>,
    requested_backend: OcrBackend,
    effective_backend: OcrBackend,
    lang: String,
    region: Rect,
    bitmap_sha256: String,
    bitmap_width: u32,
    bitmap_height: u32,
    bitmap_bytes: u64,
    result: OcrResult,
    recognition_latency_ms: u64,
    word_count: u64,
}

#[cfg(windows)]
fn read_ocr_cache_row(
    runtime: &ReflexRuntime,
    cache_key: &str,
    request: &crate::m1::ResolvedReadTextRequest,
    bitmap_width: u32,
    bitmap_height: u32,
    bitmap_sha256: &str,
) -> Result<Option<OcrCacheRow>, ErrorData> {
    let rows = runtime
        .storage_cf_prefix_rows(cf::CF_OCR_CACHE, cache_key.as_bytes(), 1)
        .map_err(|error| {
            mcp_error(
                error.code(),
                format!("OCR cache read failed for key {cache_key}: {error}"),
            )
        })?;
    let Some((row_key, value)) = rows
        .into_iter()
        .find(|(row_key, _value)| row_key.as_slice() == cache_key.as_bytes())
    else {
        return Ok(None);
    };
    let row = decode_json::<OcrCacheRow>(&value).map_err(|error| {
        mcp_error(
            error.code(),
            format!("OCR cache row decode failed for key {cache_key}: {error}"),
        )
    })?;
    if !valid_ocr_cache_row(
        &row,
        cache_key,
        request,
        bitmap_width,
        bitmap_height,
        bitmap_sha256,
    ) {
        tracing::warn!(
            code = "OCR_CACHE_ROW_INVALID",
            cache_key = %cache_key,
            row_key = %String::from_utf8_lossy(&row_key),
            "OCR cache row failed validation and will be ignored"
        );
        return Ok(None);
    }
    Ok(Some(row))
}

#[cfg(windows)]
fn valid_ocr_cache_row(
    row: &OcrCacheRow,
    cache_key: &str,
    request: &crate::m1::ResolvedReadTextRequest,
    bitmap_width: u32,
    bitmap_height: u32,
    bitmap_sha256: &str,
) -> bool {
    row.schema_version == SCHEMA_VERSION
        && row.cache_key == cache_key
        && row.requested_backend == request.requested_backend
        && row.effective_backend == request.effective_backend
        && row.lang == request.lang()
        && row.region == request.region
        && row.bitmap_width == bitmap_width
        && row.bitmap_height == bitmap_height
        && row.bitmap_sha256 == bitmap_sha256
        && row.result.region == request.region
}

#[cfg(windows)]
fn ocr_cache_key(
    request: &crate::m1::ResolvedReadTextRequest,
    bitmap_width: u32,
    bitmap_height: u32,
    bitmap_sha256: &str,
) -> String {
    format!(
        "ocr/cache/v1/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}",
        ocr_backend_name(request.requested_backend),
        ocr_backend_name(request.effective_backend),
        sha256_hex(request.lang().as_bytes()),
        request.region.x,
        request.region.y,
        request.region.w,
        request.region.h,
        bitmap_width,
        bitmap_height,
        bitmap_sha256
    )
}

#[cfg(windows)]
fn lock_reflex_runtime(
    runtime: &std::sync::Arc<std::sync::Mutex<ReflexRuntime>>,
) -> Result<std::sync::MutexGuard<'_, ReflexRuntime>, ErrorData> {
    runtime.lock().map_err(|_error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "reflex runtime lock poisoned while accessing OCR cache",
        )
    })
}

#[cfg(windows)]
fn elapsed_ms_u64(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(windows)]
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

#[cfg(windows)]
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(windows)]
const fn ocr_backend_name(backend: OcrBackend) -> &'static str {
    match backend {
        OcrBackend::Winrt => "winrt",
        OcrBackend::Crnn => "crnn",
        OcrBackend::Auto => "auto",
    }
}

#[cfg(windows)]
fn populate_profile_hud(
    input: &mut synapse_perception::ObservationInput,
    profile: &Profile,
    profile_dir: &Path,
) {
    for field in &profile.hud {
        input.hud.by_name.remove(&field.name);
        input.hud.errors.remove(&field.name);
        match extract_profile_hud_field(field, input.foreground.window_bounds, profile_dir) {
            Ok(reading) => {
                input.hud.by_name.insert(field.name.clone(), reading);
            }
            Err(error) => {
                record_hud_error(&mut input.hud, &field.name, error.code(), error.to_string());
            }
        }
    }
}

#[cfg(not(windows))]
fn populate_profile_hud(
    input: &mut synapse_perception::ObservationInput,
    profile: &Profile,
    _profile_dir: &std::path::Path,
) {
    for field in &profile.hud {
        input.hud.by_name.remove(&field.name);
        input.hud.errors.remove(&field.name);
        record_hud_error(
            &mut input.hud,
            &field.name,
            error_codes::HUD_EXTRACTION_FAILED,
            "profile HUD extraction requires Windows screen capture",
        );
    }
}

#[cfg(windows)]
fn extract_profile_hud_field(
    field: &HudFieldSpec,
    window_bounds: Rect,
    profile_dir: &Path,
) -> PerceptionResult<HudReading> {
    let screen_region = resolve_hud_region_rect(&field.region, window_bounds)?;
    let region_image = capture_region_gray(screen_region)?;
    match &field.extractor {
        HudExtractor::ColorRatio {
            sample_points: _,
            mapping,
        } => color_ratio_reading(field, screen_region, &region_image, mapping),
        HudExtractor::TemplateMatch { templates } => {
            let loaded_templates = load_templates(&field.name, templates, profile_dir)?;
            let provider = SystemOcrProvider;
            extract_field(&FieldExtractionRequest {
                field,
                screen_region,
                region_image: &region_image,
                templates: &loaded_templates,
                ocr_provider: &provider,
                stale_ms: 0,
            })
            .map(|extraction| extraction.reading)
        }
        HudExtractor::WinrtOcr | HudExtractor::Crnn { .. } => {
            let provider = HudTextProvider;
            extract_field(&FieldExtractionRequest {
                field,
                screen_region,
                region_image: &region_image,
                templates: &[],
                ocr_provider: &provider,
                stale_ms: 0,
            })
            .map(|extraction| extraction.reading)
        }
    }
}

#[cfg(windows)]
struct HudTextProvider;

#[cfg(windows)]
impl OcrProvider for HudTextProvider {
    fn read_text(&self, region: Rect) -> PerceptionResult<Vec<TextRegion>> {
        if let Some(text_region) = bounded_uia_text_region(region) {
            return Ok(vec![text_region]);
        }
        SystemOcrProvider.read_text(region)
    }
}

#[cfg(windows)]
fn bounded_uia_text_region(region: Rect) -> Option<TextRegion> {
    let point = region_center(region)?;
    let element = synapse_a11y::element_node_from_point(point).ok()?;
    let name = element.name.trim();
    if name.is_empty() {
        return None;
    }
    let bbox = element.bbox;
    if !uia_text_bbox_is_bound_to_hud_region(region, bbox) {
        return None;
    }
    Some(TextRegion {
        text: name.to_owned(),
        bbox,
        confidence: 1.0,
    })
}

#[cfg(windows)]
const fn region_center(region: Rect) -> Option<Point> {
    if region.w <= 0 || region.h <= 0 {
        return None;
    }
    Some(Point {
        x: region.x.saturating_add(region.w / 2),
        y: region.y.saturating_add(region.h / 2),
    })
}

#[cfg(windows)]
fn uia_text_bbox_is_bound_to_hud_region(region: Rect, bbox: Rect) -> bool {
    if region.w <= 0 || region.h <= 0 || bbox.w <= 0 || bbox.h <= 0 {
        return false;
    }
    let Some(region_area) = rect_area(region) else {
        return false;
    };
    let Some(bbox_area) = rect_area(bbox) else {
        return false;
    };
    bbox_area <= region_area.saturating_mul(4) && rects_intersect(region, bbox)
}

#[cfg(windows)]
fn rect_area(rect: Rect) -> Option<i64> {
    i64::from(rect.w).checked_mul(i64::from(rect.h))
}

#[cfg(windows)]
const fn rects_intersect(a: Rect, b: Rect) -> bool {
    let a_right = a.x.saturating_add(a.w);
    let a_bottom = a.y.saturating_add(a.h);
    let b_right = b.x.saturating_add(b.w);
    let b_bottom = b.y.saturating_add(b.h);
    a.x < b_right && a_right > b.x && a.y < b_bottom && a_bottom > b.y
}

#[cfg(windows)]
fn capture_region_gray(region: Rect) -> PerceptionResult<GrayImage> {
    let captured = synapse_capture::screen_region_to_bgra_bitmap(region).map_err(|error| {
        hud_error(format!(
            "HUD screen capture failed for region {region:?}: {error}"
        ))
    })?;
    bgra_to_gray(captured.width, captured.height, &captured.bytes)
}

#[cfg(windows)]
fn bgra_to_gray(width: u32, height: u32, bytes: &[u8]) -> PerceptionResult<GrayImage> {
    let expected_len = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| hud_error("HUD BGRA dimensions overflow"))?;
    let actual_len = u64::try_from(bytes.len())
        .map_err(|_err| hud_error("HUD BGRA byte length does not fit u64"))?;
    if actual_len < expected_len {
        return Err(hud_error(format!(
            "HUD BGRA buffer too short: expected at least {expected_len} bytes, got {actual_len}"
        )));
    }

    let mut image = GrayImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let idx = usize::try_from((u64::from(y) * u64::from(width) + u64::from(x)) * 4)
                .map_err(|_err| hud_error("HUD BGRA pixel offset does not fit usize"))?;
            image.put_pixel(
                x,
                y,
                Luma([bgra_luma(bytes[idx], bytes[idx + 1], bytes[idx + 2])]),
            );
        }
    }
    Ok(image)
}

#[cfg(windows)]
fn color_ratio_reading(
    field: &HudFieldSpec,
    screen_region: Rect,
    region_image: &GrayImage,
    mapping: &str,
) -> PerceptionResult<HudReading> {
    if mapping != "luma_stddev_0_1" {
        return Err(hud_error(format!(
            "unsupported color_ratio mapping {mapping:?} for HUD field {:?}",
            field.name
        )));
    }
    let score = gray_luma_stddev_0_1(region_image);
    let raw_text = format!("{score:.6}");
    let parsed = parse_hud_text(&field.parser, &raw_text)?;
    Ok(HudReading {
        raw_text: format!(
            "{raw_text} region={}x{}@{},{}",
            screen_region.w, screen_region.h, screen_region.x, screen_region.y
        ),
        parsed,
        confidence: score,
        stale_ms: 0,
    })
}

#[cfg(windows)]
fn load_templates(
    field_name: &str,
    paths: &[String],
    profile_dir: &Path,
) -> PerceptionResult<Vec<HudTemplate>> {
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let label = template_label(path, index);
            let value = template_value(field_name, path, index)?;
            let resolved = resolve_template_path(path, profile_dir);
            HudTemplate::load(label, value, resolved)
        })
        .collect()
}

#[cfg(windows)]
fn resolve_template_path(path: &str, profile_dir: &Path) -> PathBuf {
    let raw = Path::new(path);
    if raw.is_absolute() {
        return raw.to_path_buf();
    }

    let mut candidates = vec![PathBuf::from(path), profile_dir.join(path)];
    candidates.push(profile_dir.join("assets").join(path));
    if let Some(parent) = profile_dir.parent() {
        candidates.push(parent.join(path));
    }

    candidates
        .iter()
        .find(|candidate| candidate.exists())
        .cloned()
        .unwrap_or_else(|| profile_dir.join(path))
}

#[cfg(windows)]
fn template_label(path: &str, index: usize) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.trim().is_empty())
        .map_or_else(|| format!("template_{index}"), str::to_owned)
}

#[cfg(windows)]
fn template_value(field_name: &str, path: &str, index: usize) -> PerceptionResult<u32> {
    let lower_field = field_name.to_ascii_lowercase();
    let lower = path.to_ascii_lowercase();
    if lower_field.contains("hunger") {
        if lower.contains("full") || lower.contains("half") {
            return Ok(1);
        }
        if lower.contains("empty") {
            return Ok(0);
        }
    }
    if lower.contains("full") {
        return Ok(2);
    }
    if lower.contains("half") {
        return Ok(1);
    }
    if lower.contains("empty") {
        return Ok(0);
    }
    match index {
        0 => Ok(2),
        1 => Ok(1),
        2 => Ok(0),
        _ => Err(hud_error(format!(
            "cannot infer HUD template value for path {path:?}"
        ))),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::{ocr_cache_key, sha256_hex, template_value};
    use crate::m1::ResolvedReadTextRequest;
    use synapse_core::{OcrBackend, Rect};

    #[test]
    fn template_values_are_field_specific_for_minecraft_status_bars() -> Result<(), String> {
        let heart_full = template_value("minecraft.hp_hearts", "hearts/full.png", 0)
            .map_err(|error| error.to_string())?;
        let heart_half = template_value("minecraft.hp_hearts", "hearts/half.png", 1)
            .map_err(|error| error.to_string())?;
        let hunger_full = template_value("minecraft.hunger", "hunger/full.png", 0)
            .map_err(|error| error.to_string())?;
        let hunger_half = template_value("minecraft.hunger", "hunger/half.png", 1)
            .map_err(|error| error.to_string())?;
        let hunger_empty = template_value("minecraft.hunger", "hunger/empty.png", 2)
            .map_err(|error| error.to_string())?;

        assert_eq!(heart_full, 2);
        assert_eq!(heart_half, 1);
        assert_eq!(hunger_full, 1);
        assert_eq!(hunger_half, 1);
        assert_eq!(hunger_empty, 0);
        Ok(())
    }

    #[test]
    fn ocr_cache_key_changes_when_pixels_change() {
        let request = ResolvedReadTextRequest {
            region: Rect {
                x: 10,
                y: 20,
                w: 200,
                h: 80,
            },
            requested_backend: OcrBackend::Winrt,
            effective_backend: OcrBackend::Winrt,
            lang_hint: Some("en-US".to_owned()),
            synthetic: false,
        };

        let first_hash = sha256_hex(&[1, 2, 3, 4]);
        let second_hash = sha256_hex(&[1, 2, 3, 5]);

        let first = ocr_cache_key(&request, 200, 80, &first_hash);
        let second = ocr_cache_key(&request, 200, 80, &second_hash);

        assert_ne!(first, second);
        assert!(first.contains("/winrt/winrt/"));
    }

    #[test]
    fn ocr_cache_key_separates_auto_from_explicit_winrt_requests() {
        let mut explicit = ResolvedReadTextRequest {
            region: Rect {
                x: 10,
                y: 20,
                w: 200,
                h: 80,
            },
            requested_backend: OcrBackend::Winrt,
            effective_backend: OcrBackend::Winrt,
            lang_hint: None,
            synthetic: false,
        };
        let hash = sha256_hex(&[9, 9, 9, 9]);
        let explicit_key = ocr_cache_key(&explicit, 200, 80, &hash);

        explicit.requested_backend = OcrBackend::Auto;
        let auto_key = ocr_cache_key(&explicit, 200, 80, &hash);

        assert_ne!(explicit_key, auto_key);
        assert!(auto_key.contains("/auto/winrt/"));
    }
}

#[cfg(windows)]
fn gray_luma_stddev_0_1(region_image: &GrayImage) -> f32 {
    let mut count = 0.0_f32;
    let mut sum = 0.0_f32;
    let mut sum_sq = 0.0_f32;
    for pixel in region_image.pixels() {
        let luma = f32::from(pixel.0[0]);
        count += 1.0;
        sum += luma;
        sum_sq += luma * luma;
    }
    if count <= 0.0 {
        return 0.0;
    }
    let mean = sum / count;
    let variance = mean.mul_add(-mean, sum_sq / count).max(0.0);
    (variance.sqrt() / 128.0).clamp(0.0, 1.0)
}

#[cfg(windows)]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn bgra_luma(b: u8, g: u8, r: u8) -> u8 {
    let luma = 0.0722_f32.mul_add(
        f32::from(b),
        0.7152_f32.mul_add(f32::from(g), 0.2126_f32 * f32::from(r)),
    );
    luma.round().clamp(0.0, 255.0) as u8
}

#[cfg(windows)]
fn hud_error(detail: impl Into<String>) -> PerceptionError {
    PerceptionError::HudExtractionFailed {
        detail: detail.into(),
    }
}

fn record_hud_error(
    hud: &mut HudReadings,
    field_name: &str,
    code: &'static str,
    detail: impl Into<String>,
) {
    hud.errors.insert(
        field_name.to_owned(),
        HudFieldError {
            code: code.to_owned(),
            detail: detail.into(),
        },
    );
}
