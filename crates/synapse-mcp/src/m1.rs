mod detection;
mod ocr;
mod search;
mod sources;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use rmcp::{ErrorData, handler::server::common, model::JsonObject, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synapse_capture::{
    CAPTURE_CHANNEL_CAPACITY, CaptureBackend, CaptureConfig, CaptureController, CaptureTarget,
    CaptureThreadPriority, resolve_capture_target,
};
use synapse_core::{
    AccessibleNode, CaptureRuntimeReadback, ElementId, FocusedElement, ForegroundContext,
    ObservationCaptureConfig, ObservationCaptureTarget, OcrBackend, PerceptionMode, Profile,
    ProfileCapture, ProfileCaptureTarget, ProfileDetection, Rect, error_codes,
};
use synapse_perception::{ObservationInput, ObserveInclude, parse_perception_mode};

pub use detection::populate_detection_from_state;
use detection::{DetectionRuntime, DetectionRuntimeConfig, default_detection_config};
#[cfg(windows)]
pub use ocr::ocr_result_from_web_bitmap;
#[cfg(windows)]
pub use ocr::read_text_request_from_bgra;
pub use ocr::{ResolvedReadTextRequest, read_text_request_uncached, resolve_read_text_request};
use search::{element_match, entity_match};
pub use sources::{FsRecentTracker, populate_clipboard_summary, populate_fs_recent};
use sources::{
    element_input_from_id, platform_input, synthetic_notepad_input, window_input_from_hwnd,
};

pub type SharedM1State = Arc<Mutex<M1State>>;
const MIN_CAPTURE_UPDATE_INTERVAL_MS: u64 = 16;
const MIN_CAPTURE_UPDATE_INTERVAL_MS_U32: u32 = 16;

#[derive(Debug)]
pub struct M1State {
    pub capture_config: CaptureConfig,
    pub capture_controller: CaptureController,
    pub capture_generation: u64,
    pub active_capture_config: ObservationCaptureConfig,
    pub perception_mode: PerceptionMode,
    pub manual_perception_mode: Option<PerceptionMode>,
    pub detection_config: DetectionRuntimeConfig,
    pub detection_runtime: DetectionRuntime,
    pub synthetic: Option<ObservationInput>,
    pub force_no_perception: bool,
    pub force_observe_internal: bool,
    pub last_observed_foreground: Option<ForegroundContext>,
    pub everquest_log_cursor: Option<EverQuestLogCursorState>,
    pub everquest_event_seq: u64,
    pub fs_recent_tracker: FsRecentTracker,
}

impl M1State {
    #[must_use]
    pub fn from_env() -> Self {
        let synthetic = match std::env::var("SYNAPSE_MCP_SYNTHETIC_FIXTURE") {
            Ok(value) if value.eq_ignore_ascii_case("notepad") => Some(synthetic_notepad_input()),
            _ => None,
        };
        let force_no_perception = std::env::var("SYNAPSE_MCP_FORCE_NO_PERCEPTION")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
        let force_observe_internal = std::env::var("SYNAPSE_MCP_FORCE_OBSERVE_INTERNAL")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
        Self {
            capture_config: CaptureConfig::default().with_env_backend(),
            capture_controller: CaptureController::new(),
            capture_generation: 0,
            active_capture_config: default_observation_capture_config(),
            perception_mode: PerceptionMode::Auto,
            manual_perception_mode: None,
            detection_config: default_detection_config(),
            detection_runtime: DetectionRuntime::default(),
            synthetic,
            force_no_perception,
            force_observe_internal,
            last_observed_foreground: None,
            everquest_log_cursor: None,
            everquest_event_seq: 0,
            fs_recent_tracker: FsRecentTracker::from_env(),
        }
    }

    #[must_use]
    pub fn capture_runtime_readback(&self) -> CaptureRuntimeReadback {
        let Some(handle) = self.capture_controller.active() else {
            return CaptureRuntimeReadback {
                status: "inactive".to_owned(),
                target: None,
                backend: None,
                selected_backend: Some(
                    capture_backend_name(self.capture_config.selected_backend()).to_owned(),
                ),
                generation: self.capture_controller.generation(),
                min_update_interval_ms: Some(
                    u32::try_from(self.capture_config.min_update_interval_ms)
                        .unwrap_or(u32::MAX)
                        .max(MIN_CAPTURE_UPDATE_INTERVAL_MS_U32),
                ),
                cursor_visible: Some(self.capture_config.cursor_visible),
                dirty_region_only: Some(self.capture_config.dirty_region_only),
                frames_captured: 0,
                frames_dropped: 0,
                latest_frame_seq: None,
                latest_frame_width: None,
                latest_frame_height: None,
                channel_len: 0,
                channel_capacity: CAPTURE_CHANNEL_CAPACITY,
                thread_priority: None,
                stop_requested: false,
            };
        };

        let stats = handle.stats();
        let active_config = handle.config();
        let latest_frame = stats.latest_frame();
        CaptureRuntimeReadback {
            status: "running".to_owned(),
            target: Some(observation_target_from_capture_target(
                &handle.target().target,
            )),
            backend: stats
                .effective_backend()
                .map(|backend| capture_backend_name(backend).to_owned()),
            selected_backend: Some(capture_backend_name(handle.target().backend).to_owned()),
            generation: self.capture_controller.generation(),
            min_update_interval_ms: Some(
                u32::try_from(active_config.min_update_interval_ms)
                    .unwrap_or(u32::MAX)
                    .max(MIN_CAPTURE_UPDATE_INTERVAL_MS_U32),
            ),
            cursor_visible: Some(active_config.cursor_visible),
            dirty_region_only: Some(active_config.dirty_region_only),
            frames_captured: stats.frames_captured(),
            frames_dropped: stats.frames_dropped(),
            latest_frame_seq: latest_frame.map(|frame| frame.frame_seq),
            latest_frame_width: latest_frame.map(|frame| frame.width),
            latest_frame_height: latest_frame.map(|frame| frame.height),
            channel_len: handle.channel_len(),
            channel_capacity: handle.channel_capacity(),
            thread_priority: Some(capture_thread_priority_name(stats.thread_priority())),
            stop_requested: handle.is_stop_requested(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EverQuestLogCursorState {
    pub path: PathBuf,
    pub offset: u64,
}

impl Default for M1State {
    fn default() -> Self {
        Self::from_env()
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObserveParams {
    #[serde(default)]
    pub include: Vec<ObserveSlot>,
    #[serde(default)]
    pub depth: Option<u32>,
    #[serde(default)]
    pub max_elements: Option<usize>,
    #[serde(default)]
    pub element_offset: Option<usize>,
    #[serde(default)]
    pub subtree_root: Option<ElementId>,
    #[serde(default)]
    pub since_event_seq: Option<u64>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObserveSlot {
    Focused,
    Elements,
    Entities,
    Hud,
    Audio,
    Events,
    Clipboard,
    Fs,
    Diagnostics,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindParams {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub name_substring: Option<String>,
    #[serde(default)]
    pub automation_id: Option<String>,
    #[serde(default)]
    pub scope: Option<FindScope>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub in_window: Option<ElementId>,
    #[serde(default)]
    pub window_hwnd: Option<i64>,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindScope {
    Elements,
    Entities,
    #[default]
    Both,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindResponse {
    pub results: Vec<FindResult>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindResult {
    pub kind: FindResultKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<ElementId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_label: Option<String>,
    pub bbox: Rect,
    pub score: f32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindResultKind {
    Element,
    Entity,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadTextParams {
    #[serde(default)]
    pub region: Option<Rect>,
    #[serde(default)]
    pub element_id: Option<ElementId>,
    #[serde(default)]
    pub backend: OcrBackend,
    #[serde(default)]
    pub lang_hint: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureScreenshotParams {
    pub path: String,
    #[serde(default)]
    pub region: Option<Rect>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureScreenshotResponse {
    pub path: String,
    pub format: CaptureScreenshotFormat,
    pub capture_backend: String,
    pub region: Rect,
    pub width: u32,
    pub height: u32,
    pub bytes_written: u64,
    pub bitmap_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<ForegroundContext>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptureScreenshotFormat {
    Png,
    Jpeg,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCaptureTargetParams {
    pub target: CaptureTargetParam,
    #[serde(default)]
    pub min_update_interval_ms: Option<u64>,
    #[serde(default)]
    pub cursor_visible: Option<bool>,
    #[serde(default)]
    pub dirty_region_only: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureTargetParam {
    Primary,
    Monitor { monitor_index: u32 },
    Window { window_hwnd: i64 },
    ElementWindow { element_id: ElementId },
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCaptureTargetResponse {
    pub previous: CaptureTargetWire,
    pub current: CaptureTargetWire,
    pub generation: u64,
    pub backend: String,
    pub capture_runtime: CaptureRuntimeReadback,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureTargetWire {
    Primary,
    Monitor { monitor_index: u32 },
    Window { window_hwnd: i64 },
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPerceptionModeParams {
    pub mode: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPerceptionModeResponse {
    pub previous: PerceptionMode,
    pub mode: PerceptionMode,
    pub rationale: String,
}

pub fn empty_input_schema() -> Arc<JsonObject> {
    common::schema_for_type::<EmptyParams>()
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

#[must_use]
pub fn observe_include(params: &ObserveParams) -> ObserveInclude {
    let mut include = if params.include.is_empty() {
        ObserveInclude::default()
    } else {
        ObserveInclude {
            focused: false,
            elements: false,
            entities: false,
            hud: false,
            audio: false,
            events: false,
            clipboard: false,
            fs: false,
            diagnostics: false,
            max_subtree_depth: 2,
            max_subtree_nodes: 60,
            element_offset: 0,
            max_entities: 60,
        }
    };
    for slot in &params.include {
        match slot {
            ObserveSlot::Focused => include.focused = true,
            ObserveSlot::Elements => include.elements = true,
            ObserveSlot::Entities => include.entities = true,
            ObserveSlot::Hud => include.hud = true,
            ObserveSlot::Audio => include.audio = true,
            ObserveSlot::Events => include.events = true,
            ObserveSlot::Clipboard => include.clipboard = true,
            ObserveSlot::Fs => include.fs = true,
            ObserveSlot::Diagnostics => include.diagnostics = true,
        }
    }
    include.max_subtree_depth = params.depth.unwrap_or(2).min(6);
    include.max_subtree_nodes = params.max_elements.unwrap_or(60).clamp(1, 500);
    include.element_offset = params.element_offset.unwrap_or(0).min(100_000);
    include
}

pub fn current_input(state: &M1State, depth: u32) -> Result<ObservationInput, ErrorData> {
    if state.force_observe_internal {
        return Err(mcp_error(
            error_codes::OBSERVE_INTERNAL,
            "forced observe internal error",
        ));
    }
    if state.force_no_perception {
        return Err(mcp_error(
            error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
            "no perception source is available",
        ));
    }
    if let Some(input) = &state.synthetic {
        let mut input = input_limited_to_depth(input.clone(), depth);
        if state.perception_mode != PerceptionMode::Auto {
            input.mode_override = Some(state.perception_mode);
        }
        input.capture_config = Some(state.active_capture_config.clone());
        input.capture_runtime = Some(state.capture_runtime_readback());
        return Ok(input);
    }
    let mut input = platform_input(depth, state.perception_mode)?;
    input.capture_config = Some(state.active_capture_config.clone());
    input.capture_runtime = Some(state.capture_runtime_readback());
    Ok(input)
}

pub fn observe_input(
    state: &M1State,
    params: &ObserveParams,
) -> Result<ObservationInput, ErrorData> {
    let depth = params.depth.unwrap_or(2).min(6);
    if let Some(element_id) = &params.subtree_root {
        return element_input_from_id(element_id, depth, state.perception_mode);
    }
    current_input(state, depth)
}

/// Attaches CDP (when reachable) and folds the page's DOM/accessibility tree
/// into `input.elements` as queryable web nodes (#685), upgrading `web_path` to
/// `cdp`. This is the async companion to the synchronous probe in
/// `sources::populate_cdp_diagnostics`: the probe reports *whether* a debug port
/// is reachable; this turns a reachable port into actual web content.
///
/// Fail-loud: an attach/tree failure flips `cdp.status` to `attach_failed` with
/// the specific reason code and detail, and leaves `web_path = uia_only` — never
/// a silent empty tree. Non-browser / no-port foregrounds are a no-op.
#[cfg(windows)]
pub async fn enrich_input_with_cdp(input: &mut ObservationInput, max_depth: u32, max_nodes: usize) {
    use synapse_core::{CdpStatus, WebPerceptionPath};

    let Some(cdp) = input.cdp.clone() else {
        return;
    };
    if cdp.status != CdpStatus::Ok {
        return;
    }
    let Some(endpoint) = cdp.endpoint.clone() else {
        return;
    };
    let hwnd = input.foreground.hwnd;
    let title = input.foreground.window_title.clone();

    match synapse_a11y::fetch_dom_snapshot(&endpoint, hwnd, &title, max_nodes).await {
        Ok(snapshot) => {
            let count = u32::try_from(snapshot.nodes.len()).unwrap_or(u32::MAX);
            for mut node in snapshot.nodes {
                // Clamp web-node depth to the requested observe depth so deeply
                // nested DOM elements still survive the element depth filter;
                // parent links keep the true hierarchy.
                node.depth = node.depth.min(max_depth);
                input.elements.push(node);
            }
            input.web_path = Some(WebPerceptionPath::Cdp);
            if let Some(diagnostics) = input.cdp.as_mut() {
                diagnostics.attached_node_count = Some(count);
            }
            tracing::info!(
                code = "A11Y_CDP_DOM_ATTACHED",
                endpoint = %endpoint,
                hwnd,
                page_url = %snapshot.page_url,
                node_count = count,
                total_ax_nodes = snapshot.total_ax_nodes,
                "attached CDP DOM tree into observation elements"
            );
        }
        Err(error) => {
            tracing::error!(
                code = error.code(),
                endpoint = %endpoint,
                hwnd,
                error = %error,
                "CDP DOM snapshot failed; web content not exposed (web_path stays uia_only)"
            );
            if let Some(diagnostics) = input.cdp.as_mut() {
                diagnostics.status = CdpStatus::AttachFailed;
                diagnostics.reason_code = Some(error.code().to_owned());
                diagnostics.detail = Some(error.to_string());
            }
        }
    }
}

const BROWSER_OCR_CHROME_TOP_PX: i32 = 96;
const BROWSER_OCR_TILE_HEIGHT_PX: i32 = 600;
const BROWSER_OCR_MAX_TILES: usize = 8;
const OCR_RUNTIME_RECT_PREFIX: &str = "0c0c01";

/// Recovers browser page text through screen OCR when CDP did not yield a DOM
/// tree. This is the degraded leg of the #687 ladder: it creates queryable text
/// nodes and upgrades `web_path` to `ocr` only when OCR actually returned text.
#[cfg(windows)]
pub fn enrich_input_with_browser_ocr(input: &mut ObservationInput, max_nodes: usize) {
    use synapse_core::{SensorStatus, WebPerceptionPath};

    if max_nodes == 0 || !should_attempt_browser_ocr(input) {
        return;
    }
    if !matches!(
        input.capture_status,
        SensorStatus::Healthy | SensorStatus::DegradedLatency { .. }
    ) {
        tracing::warn!(
            code = "A11Y_BROWSER_OCR_SKIPPED_CAPTURE_UNAVAILABLE",
            hwnd = input.foreground.hwnd,
            capture_status = ?input.capture_status,
            "browser OCR fallback skipped because screen capture is not available"
        );
        return;
    }
    let Some(content_region) = browser_content_region(input.foreground.window_bounds) else {
        tracing::warn!(
            code = "A11Y_BROWSER_OCR_SKIPPED_EMPTY_REGION",
            hwnd = input.foreground.hwnd,
            window_bounds = ?input.foreground.window_bounds,
            "browser OCR fallback skipped because the browser content region is empty"
        );
        return;
    };

    let started = std::time::Instant::now();
    let mut words = Vec::new();
    let mut attempted_tiles = 0usize;
    let mut failed_tiles = 0usize;
    for tile in browser_ocr_tiles(content_region) {
        attempted_tiles += 1;
        match synapse_perception::read_text(tile) {
            Ok(mut tile_words) => words.append(&mut tile_words),
            Err(error) => {
                failed_tiles = failed_tiles.saturating_add(1);
                tracing::debug!(
                    code = error.code(),
                    hwnd = input.foreground.hwnd,
                    tile = ?tile,
                    error = %error,
                    "browser OCR tile produced no text"
                );
            }
        }
        if words.len() >= max_nodes {
            break;
        }
    }
    input
        .sensor_latency_ms
        .insert("ocr".to_owned(), started.elapsed().as_secs_f32() * 1000.0);

    let added = apply_browser_ocr_words(input, words, max_nodes);
    if added == 0 {
        tracing::warn!(
            code = "A11Y_BROWSER_OCR_NO_TEXT",
            hwnd = input.foreground.hwnd,
            content_region = ?content_region,
            attempted_tiles,
            failed_tiles,
            "browser OCR fallback found no readable page text; web_path remains uia_only"
        );
        return;
    }
    tracing::info!(
        code = "A11Y_BROWSER_OCR_ATTACHED",
        hwnd = input.foreground.hwnd,
        content_region = ?content_region,
        attempted_tiles,
        failed_tiles,
        node_count = added,
        web_path = %WebPerceptionPath::Ocr.as_str(),
        "browser OCR fallback added queryable text nodes"
    );
}

#[cfg(not(windows))]
pub fn enrich_input_with_browser_ocr(_input: &mut ObservationInput, _max_nodes: usize) {}

fn should_attempt_browser_ocr(input: &ObservationInput) -> bool {
    use synapse_core::{CdpStatus, WebPerceptionPath};

    if input.web_path != Some(WebPerceptionPath::UiaOnly) {
        return false;
    }
    if has_chromium_main_pane_uia_content(input) {
        return false;
    }
    input.cdp.as_ref().is_some_and(|diagnostics| {
        matches!(
            diagnostics.status,
            CdpStatus::Unreachable | CdpStatus::AttachFailed
        )
    })
}

fn has_chromium_main_pane_uia_content(input: &ObservationInput) -> bool {
    if !synapse_a11y::is_chromium_family(&input.foreground.process_name) {
        return false;
    }
    let Some(main_region) = browser_main_pane_region(input.foreground.window_bounds) else {
        return false;
    };
    input.elements.iter().any(|node| {
        if node
            .automation_id
            .as_deref()
            .is_some_and(|automation_id| automation_id.starts_with("ocr:"))
        {
            return false;
        }
        if node.bbox.w <= 0 || node.bbox.h <= 0 || !rects_overlap(node.bbox, main_region) {
            return false;
        }
        let role = node.role.to_ascii_lowercase();
        matches!(
            role.as_str(),
            "button"
                | "cell"
                | "data_item"
                | "edit"
                | "heading"
                | "hyperlink"
                | "link"
                | "list_item"
                | "row"
                | "text"
        ) && (!node.name.trim().is_empty()
            || node
                .value
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || !node.patterns.is_empty()
            || role == "document")
    })
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    let a_right = a.x.saturating_add(a.w);
    let a_bottom = a.y.saturating_add(a.h);
    let b_right = b.x.saturating_add(b.w);
    let b_bottom = b.y.saturating_add(b.h);
    a.x < b_right && a_right > b.x && a.y < b_bottom && a_bottom > b.y
}

fn apply_browser_ocr_words(
    input: &mut ObservationInput,
    words: Vec<synapse_perception::TextRegion>,
    max_nodes: usize,
) -> usize {
    let nodes = browser_ocr_nodes(input.foreground.hwnd, words, max_nodes);
    if nodes.is_empty() {
        return 0;
    }
    let added = nodes.len();
    input.elements.extend(nodes);
    input.web_path = Some(synapse_core::WebPerceptionPath::Ocr);
    added
}

fn browser_ocr_nodes(
    hwnd: i64,
    words: Vec<synapse_perception::TextRegion>,
    max_nodes: usize,
) -> Vec<AccessibleNode> {
    words
        .into_iter()
        .filter(|word| !word.text.trim().is_empty() && word.bbox.w > 0 && word.bbox.h > 0)
        .take(max_nodes)
        .enumerate()
        .map(|(index, word)| {
            let trimmed = word.text.trim().to_owned();
            let bbox = word.bbox;
            AccessibleNode {
                element_id: browser_ocr_element_id(hwnd, bbox, index),
                parent: None,
                name: trimmed,
                role: "text".to_owned(),
                automation_id: Some(format!("ocr:word:{index}")),
                value: None,
                bbox,
                enabled: true,
                focused: false,
                patterns: Vec::new(),
                children_count: 0,
                depth: 1,
            }
        })
        .collect()
}

fn browser_ocr_element_id(hwnd: i64, bbox: Rect, index: usize) -> ElementId {
    synapse_core::element_id(
        hwnd,
        &format!(
            "{OCR_RUNTIME_RECT_PREFIX}{:08x}{:08x}{:08x}{:08x}{index:08x}",
            bbox.x.cast_unsigned(),
            bbox.y.cast_unsigned(),
            bbox.w.cast_unsigned(),
            bbox.h.cast_unsigned()
        ),
    )
}

pub(crate) fn browser_ocr_rect_from_element_id(element_id: &ElementId) -> Option<Rect> {
    let parts = element_id.parts().ok()?;
    browser_ocr_rect_from_runtime_id(&parts.runtime_id_hex)
}

pub(crate) fn is_browser_ocr_element_id(element_id: &ElementId) -> bool {
    element_id
        .parts()
        .is_ok_and(|parts| parts.runtime_id_hex.starts_with(OCR_RUNTIME_RECT_PREFIX))
}

fn browser_ocr_rect_from_runtime_id(runtime_id_hex: &str) -> Option<Rect> {
    let encoded = runtime_id_hex.strip_prefix(OCR_RUNTIME_RECT_PREFIX)?;
    if encoded.len() < 32 {
        return None;
    }
    let x = i32_from_hex_u32(&encoded[0..8])?;
    let y = i32_from_hex_u32(&encoded[8..16])?;
    let w = i32_from_hex_u32(&encoded[16..24])?;
    let h = i32_from_hex_u32(&encoded[24..32])?;
    (w > 0 && h > 0).then_some(Rect { x, y, w, h })
}

fn i32_from_hex_u32(raw: &str) -> Option<i32> {
    u32::from_str_radix(raw, 16).ok().map(u32::cast_signed)
}

fn browser_content_region(window_bounds: Rect) -> Option<Rect> {
    if window_bounds.w <= 0 || window_bounds.h <= 0 {
        return None;
    }
    let top_inset = if window_bounds.h > 240 {
        BROWSER_OCR_CHROME_TOP_PX.min(window_bounds.h / 3)
    } else {
        0
    };
    let height = window_bounds.h.saturating_sub(top_inset);
    (height > 0).then_some(Rect {
        x: window_bounds.x,
        y: window_bounds.y.saturating_add(top_inset),
        w: window_bounds.w,
        h: height,
    })
}

fn browser_main_pane_region(window_bounds: Rect) -> Option<Rect> {
    let content = browser_content_region(window_bounds)?;
    if content.w <= 1 {
        return Some(content);
    }
    let left_inset = (content.w / 4).clamp(240, 720).min(content.w - 1);
    Some(Rect {
        x: content.x.saturating_add(left_inset),
        y: content.y,
        w: content.w.saturating_sub(left_inset),
        h: content.h,
    })
}

fn browser_ocr_tiles(content_region: Rect) -> Vec<Rect> {
    if content_region.w <= 0 || content_region.h <= 0 {
        return Vec::new();
    }
    let mut tiles = Vec::new();
    let tile_height = content_region.h.min(BROWSER_OCR_TILE_HEIGHT_PX).max(1);
    let bottom = content_region.y.saturating_add(content_region.h);
    let mut y = content_region.y;
    while y < bottom && tiles.len() < BROWSER_OCR_MAX_TILES {
        let height = bottom.saturating_sub(y).min(tile_height).max(1);
        tiles.push(Rect {
            x: content_region.x,
            y,
            w: content_region.w,
            h: height,
        });
        y = y.saturating_add(height);
    }
    tiles
}

#[cfg(not(windows))]
#[allow(clippy::unused_async)]
pub async fn enrich_input_with_cdp(
    _input: &mut ObservationInput,
    _max_depth: u32,
    _max_nodes: usize,
) {
}

fn input_limited_to_depth(mut input: ObservationInput, depth: u32) -> ObservationInput {
    input.elements.retain(|node| node.depth <= depth);
    if let Some(focused) = &input.focused {
        let focused_present = input
            .elements
            .iter()
            .any(|node| node.element_id == focused.element_id);
        if focused_present {
            return input;
        }
    }
    input.focused = input.elements.first().map(focused_from_accessible_node);
    input
}

fn focused_from_accessible_node(node: &AccessibleNode) -> FocusedElement {
    FocusedElement {
        element_id: node.element_id.clone(),
        name: node.name.clone(),
        role: node.role.clone(),
        automation_id: node.automation_id.clone(),
        bbox: node.bbox,
        enabled: node.enabled,
        patterns: node.patterns.clone(),
        value: node.value.clone(),
        selected_text: None,
    }
}

/// Depth `find` walks the foreground tree. `observe`'s default is shallow (2),
/// but `find` must reach deeply-nested controls (e.g. a UWP app's display text
/// at depth ~5, or toolbar tool buttons), so it requests a deep snapshot. The
/// snapshot's node-budget/deadline bounds the cost.
const FIND_SNAPSHOT_DEPTH: u32 = 16;

/// Upper bound on CDP web nodes folded into a `find` snapshot. Web pages have
/// far more nodes than native windows, and `find` walks deeper than `observe`.
const FIND_CDP_MAX_NODES: usize = 300;

/// Builds the perception input a `find` query searches (foreground or a specific
/// window), including detection entities. Split from matching so the async `find`
/// handler can fold in CDP web nodes (#685) before matching.
pub fn build_find_input(
    state: &mut M1State,
    params: &FindParams,
) -> Result<ObservationInput, ErrorData> {
    let mut input = if let Some(hwnd) = params.window_hwnd {
        let mut input = window_input_from_hwnd(hwnd, FIND_SNAPSHOT_DEPTH, state.perception_mode)?;
        input.capture_config = Some(state.active_capture_config.clone());
        input.capture_runtime = Some(state.capture_runtime_readback());
        input
    } else {
        current_input(state, FIND_SNAPSHOT_DEPTH)?
    };
    populate_detection_from_state(state, &mut input);
    Ok(input)
}

/// Maximum CDP web nodes a `find` query folds in. Exposed so the async handler
/// can size its enrichment to match `find`'s deep snapshot.
#[must_use]
pub const fn find_cdp_max_nodes() -> usize {
    FIND_CDP_MAX_NODES
}

/// `find`'s snapshot depth (deep, so nested controls are reachable).
#[must_use]
pub const fn find_snapshot_depth() -> u32 {
    FIND_SNAPSHOT_DEPTH
}

/// Matches a prepared input against the `find` query.
#[must_use]
pub fn match_find_input(input: &ObservationInput, params: &FindParams) -> FindResponse {
    let limit = params.limit.unwrap_or(5).clamp(1, 20);
    let mut results = Vec::new();
    if matches!(
        params.scope.unwrap_or_default(),
        FindScope::Elements | FindScope::Both
    ) {
        results.extend(
            input
                .elements
                .iter()
                .filter_map(|node| element_match(node, params)),
        );
    }
    if matches!(
        params.scope.unwrap_or_default(),
        FindScope::Entities | FindScope::Both
    ) {
        results.extend(
            input
                .entities
                .iter()
                .filter_map(|entity| entity_match(entity, params)),
        );
    }
    results.sort_by(|left, right| right.score.total_cmp(&left.score));
    results.truncate(limit);
    FindResponse { results }
}

pub fn set_capture_target_in_state(
    state: &mut M1State,
    params: SetCaptureTargetParams,
) -> Result<SetCaptureTargetResponse, ErrorData> {
    let previous = capture_target_wire(&state.capture_config.target);
    let mut config = state.capture_config.clone();
    config.target = capture_target_from_param(params.target)?;
    if let Some(interval) = params.min_update_interval_ms {
        config.min_update_interval_ms = clamp_capture_interval(interval);
    }
    if let Some(cursor_visible) = params.cursor_visible {
        config.cursor_visible = cursor_visible;
    }
    if let Some(dirty_region_only) = params.dirty_region_only {
        config.dirty_region_only = dirty_region_only;
    }
    let resolved =
        resolve_capture_target(&config).map_err(|err| mcp_error(err.code(), err.to_string()))?;
    let generation = state
        .capture_controller
        .switch_to(config.clone())
        .map_err(|err| mcp_error(err.code(), err.to_string()))?;
    state.capture_config = config;
    state.capture_generation = generation;
    state.active_capture_config = observation_capture_from_capture_config(
        &state.capture_config,
        state.capture_generation,
        "manual".to_owned(),
    );
    Ok(SetCaptureTargetResponse {
        previous,
        current: capture_target_wire(&resolved.target),
        generation: state.capture_generation,
        backend: capture_backend_name(resolved.backend).to_owned(),
        capture_runtime: state.capture_runtime_readback(),
    })
}

pub fn apply_profile_runtime_config_in_state(
    state: &mut M1State,
    profile: &Profile,
) -> Result<ObservationCaptureConfig, ErrorData> {
    if state.manual_perception_mode.is_none() {
        state.perception_mode = profile.mode;
    }
    state.detection_config = detection_config_from_profile(&profile.detection);

    let mut config = state.capture_config.clone();
    config.min_update_interval_ms = u64::from(
        profile
            .capture
            .min_update_interval_ms
            .max(MIN_CAPTURE_UPDATE_INTERVAL_MS_U32),
    );
    config.cursor_visible = profile.capture.cursor_visible;
    if let Some(target) = capture_target_from_profile_target(&profile.capture.target) {
        config.target = target;
        resolve_capture_target(&config).map_err(|err| mcp_error(err.code(), err.to_string()))?;
        state.capture_config.target = config.target.clone();
    }
    state.capture_config.min_update_interval_ms = config.min_update_interval_ms;
    state.capture_config.cursor_visible = config.cursor_visible;

    let mut active_capture = observation_capture_from_profile_capture(
        &profile.capture,
        state.capture_config.dirty_region_only,
        state.capture_generation,
        format!("profile:{}", profile.id),
    );
    if capture_config_without_generation_eq(&state.active_capture_config, &active_capture) {
        active_capture.generation = state.active_capture_config.generation;
    } else {
        state.capture_generation = state.capture_generation.saturating_add(1);
        active_capture.generation = state.capture_generation;
    }
    state.active_capture_config = active_capture.clone();
    Ok(active_capture)
}

pub fn set_perception_mode_in_state(
    state: &mut M1State,
    params: &SetPerceptionModeParams,
) -> Result<SetPerceptionModeResponse, ErrorData> {
    let previous = state.perception_mode;
    let mode = parse_perception_mode(&params.mode)
        .map_err(|err| mcp_error(err.code(), err.to_string()))?;
    state.perception_mode = mode;
    state.manual_perception_mode = (mode != PerceptionMode::Auto).then_some(mode);
    Ok(SetPerceptionModeResponse {
        previous,
        mode,
        rationale: mode_rationale(mode).to_owned(),
    })
}

fn detection_config_from_profile(profile: &ProfileDetection) -> DetectionRuntimeConfig {
    DetectionRuntimeConfig::from_profile(profile)
}

pub fn mcp_error(code: &'static str, message: impl Into<String>) -> ErrorData {
    let message = message.into();
    ErrorData::new(
        rmcp::model::ErrorCode(-32099),
        message,
        Some(json!({ "code": code })),
    )
}

fn default_observation_capture_config() -> ObservationCaptureConfig {
    observation_capture_from_capture_config(&CaptureConfig::default(), 0, "default".to_owned())
}

fn observation_capture_from_capture_config(
    config: &CaptureConfig,
    generation: u64,
    source: String,
) -> ObservationCaptureConfig {
    ObservationCaptureConfig {
        target: observation_target_from_capture_target(&config.target),
        min_update_interval_ms: u32::try_from(config.min_update_interval_ms)
            .unwrap_or(u32::MAX)
            .max(MIN_CAPTURE_UPDATE_INTERVAL_MS_U32),
        cursor_visible: config.cursor_visible,
        dirty_region_only: config.dirty_region_only,
        generation,
        source,
    }
}

fn observation_capture_from_profile_capture(
    capture: &ProfileCapture,
    dirty_region_only: bool,
    generation: u64,
    source: String,
) -> ObservationCaptureConfig {
    ObservationCaptureConfig {
        target: observation_target_from_profile_target(&capture.target),
        min_update_interval_ms: capture
            .min_update_interval_ms
            .max(MIN_CAPTURE_UPDATE_INTERVAL_MS_U32),
        cursor_visible: capture.cursor_visible,
        dirty_region_only,
        generation,
        source,
    }
}

const fn observation_target_from_capture_target(
    target: &CaptureTarget,
) -> ObservationCaptureTarget {
    match target {
        CaptureTarget::Primary => ObservationCaptureTarget::PrimaryMonitor,
        CaptureTarget::Monitor { monitor_index } => ObservationCaptureTarget::MonitorIndex {
            index: *monitor_index,
        },
        CaptureTarget::Window { hwnd } => ObservationCaptureTarget::Window { window_hwnd: *hwnd },
    }
}

const fn observation_target_from_profile_target(
    target: &ProfileCaptureTarget,
) -> ObservationCaptureTarget {
    match target {
        ProfileCaptureTarget::ForegroundWindow => ObservationCaptureTarget::ForegroundWindow,
        ProfileCaptureTarget::PrimaryMonitor => ObservationCaptureTarget::PrimaryMonitor,
        ProfileCaptureTarget::MonitorIndex { index } => {
            ObservationCaptureTarget::MonitorIndex { index: *index }
        }
    }
}

const fn capture_target_from_profile_target(
    target: &ProfileCaptureTarget,
) -> Option<CaptureTarget> {
    match target {
        ProfileCaptureTarget::ForegroundWindow => None,
        ProfileCaptureTarget::PrimaryMonitor => Some(CaptureTarget::Primary),
        ProfileCaptureTarget::MonitorIndex { index } => Some(CaptureTarget::Monitor {
            monitor_index: *index,
        }),
    }
}

fn capture_config_without_generation_eq(
    left: &ObservationCaptureConfig,
    right: &ObservationCaptureConfig,
) -> bool {
    left.target == right.target
        && left.min_update_interval_ms == right.min_update_interval_ms
        && left.cursor_visible == right.cursor_visible
        && left.dirty_region_only == right.dirty_region_only
        && left.source == right.source
}

const fn clamp_capture_interval(interval_ms: u64) -> u64 {
    if interval_ms < MIN_CAPTURE_UPDATE_INTERVAL_MS {
        MIN_CAPTURE_UPDATE_INTERVAL_MS
    } else {
        interval_ms
    }
}

fn capture_target_from_param(param: CaptureTargetParam) -> Result<CaptureTarget, ErrorData> {
    match param {
        CaptureTargetParam::Primary => Ok(CaptureTarget::Primary),
        CaptureTargetParam::Monitor { monitor_index } => {
            Ok(CaptureTarget::Monitor { monitor_index })
        }
        CaptureTargetParam::Window { window_hwnd } => {
            Ok(CaptureTarget::Window { hwnd: window_hwnd })
        }
        CaptureTargetParam::ElementWindow { element_id } => {
            let rect = synapse_a11y::element_bounding_rect(&element_id).map_err(|err| {
                mcp_error(
                    error_codes::CAPTURE_TARGET_INVALID,
                    format!("element_window target could not be re-resolved: {err}"),
                )
            })?;
            validate_element_window_rect(&element_id, rect)?;
            element_id
                .parts()
                .map(|parts| CaptureTarget::Window { hwnd: parts.hwnd })
                .map_err(|err| mcp_error(error_codes::CAPTURE_TARGET_INVALID, err.to_string()))
        }
    }
}

fn validate_element_window_rect(element_id: &ElementId, rect: Rect) -> Result<(), ErrorData> {
    if rect.w <= 0 || rect.h <= 0 {
        return Err(mcp_error(
            error_codes::CAPTURE_TARGET_INVALID,
            format!(
                "element_window target is not displaying a non-empty UI rectangle: element_id={element_id} bbox=({}, {}, {}, {})",
                rect.x, rect.y, rect.w, rect.h
            ),
        ));
    }

    Ok(())
}

const fn capture_target_wire(target: &CaptureTarget) -> CaptureTargetWire {
    match target {
        CaptureTarget::Primary => CaptureTargetWire::Primary,
        CaptureTarget::Monitor { monitor_index } => CaptureTargetWire::Monitor {
            monitor_index: *monitor_index,
        },
        CaptureTarget::Window { hwnd } => CaptureTargetWire::Window { window_hwnd: *hwnd },
    }
}

const fn capture_backend_name(backend: CaptureBackend) -> &'static str {
    match backend {
        CaptureBackend::GraphicsCaptureApi => "graphics_capture_api",
        CaptureBackend::DxgiDuplication => "dxgi_duplication",
    }
}

fn capture_thread_priority_name(priority: CaptureThreadPriority) -> String {
    match priority {
        CaptureThreadPriority::TimeCritical => "time_critical".to_owned(),
        CaptureThreadPriority::Unsupported => "unsupported".to_owned(),
        CaptureThreadPriority::Unknown => "unknown".to_owned(),
        CaptureThreadPriority::Other(value) => format!("other:{value}"),
    }
}

const fn mode_rationale(mode: PerceptionMode) -> &'static str {
    match mode {
        PerceptionMode::Auto => "auto_select_by_foreground_and_a11y_density",
        PerceptionMode::A11yOnly => "manual_a11y_only",
        PerceptionMode::PixelOnly => "manual_pixel_only",
        PerceptionMode::Hybrid => "manual_hybrid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use synapse_core::{
        Backend, CdpDiagnostics, CdpStatus, ProfileBackends, ProfileDetection, ProfileMatch,
        ProfileOcr, ProfileUseScope, SensorStatus, WebPerceptionPath,
    };
    use synapse_perception::TextRegion;

    #[test]
    fn capture_interval_floor_applies_to_manual_and_profile_metadata() {
        let config = CaptureConfig {
            min_update_interval_ms: 1,
            ..CaptureConfig::default()
        };
        let manual = observation_capture_from_capture_config(&config, 42, "manual-test".to_owned());
        assert_eq!(
            manual.min_update_interval_ms,
            MIN_CAPTURE_UPDATE_INTERVAL_MS_U32
        );

        let profile = ProfileCapture {
            target: ProfileCaptureTarget::PrimaryMonitor,
            min_update_interval_ms: 1,
            cursor_visible: true,
        };
        let from_profile =
            observation_capture_from_profile_capture(&profile, true, 43, "profile:test".to_owned());
        assert_eq!(
            from_profile.min_update_interval_ms,
            MIN_CAPTURE_UPDATE_INTERVAL_MS_U32
        );
    }

    #[test]
    fn inactive_capture_runtime_readback_reports_controller_state() {
        let mut state = M1State::default();
        state.capture_config.min_update_interval_ms = 1;

        let readback = state.capture_runtime_readback();

        assert_eq!(readback.status, "inactive");
        assert!(readback.target.is_none());
        assert!(readback.backend.is_none());
        assert_eq!(readback.generation, 0);
        assert_eq!(
            readback.min_update_interval_ms,
            Some(MIN_CAPTURE_UPDATE_INTERVAL_MS_U32)
        );
        assert_eq!(readback.frames_captured, 0);
        assert_eq!(readback.frames_dropped, 0);
        assert_eq!(readback.channel_len, 0);
        assert_eq!(readback.channel_capacity, CAPTURE_CHANNEL_CAPACITY);
        assert!(!readback.stop_requested);
    }

    #[test]
    fn element_window_rect_validation_requires_non_empty_bounds() {
        let element_id = ElementId::parse("0x1:00000001").expect("valid element id");
        let positive = Rect {
            x: 10,
            y: 20,
            w: 1,
            h: 1,
        };
        assert!(validate_element_window_rect(&element_id, positive).is_ok());

        for rect in [
            Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 10,
            },
            Rect {
                x: 0,
                y: 0,
                w: 10,
                h: 0,
            },
            Rect {
                x: 0,
                y: 0,
                w: -1,
                h: 10,
            },
            Rect {
                x: 0,
                y: 0,
                w: 10,
                h: -1,
            },
        ] {
            let error = validate_element_window_rect(&element_id, rect)
                .expect_err("empty element_window bounds must fail closed");
            assert!(error.message.contains("non-empty UI rectangle"));
            assert_eq!(
                error.data.as_ref().and_then(|data| data.get("code")),
                Some(&json!(error_codes::CAPTURE_TARGET_INVALID))
            );
        }
    }

    #[test]
    fn manual_perception_mode_survives_profile_runtime_apply() {
        let mut state = M1State::default();
        set_perception_mode_in_state(
            &mut state,
            &SetPerceptionModeParams {
                mode: "pixel_only".to_owned(),
            },
        )
        .expect("manual mode parses");

        apply_profile_runtime_config_in_state(
            &mut state,
            &profile_with_mode(PerceptionMode::Hybrid),
        )
        .expect("profile config applies");

        assert_eq!(state.perception_mode, PerceptionMode::PixelOnly);
        assert_eq!(
            state.manual_perception_mode,
            Some(PerceptionMode::PixelOnly)
        );
    }

    #[test]
    fn auto_perception_mode_releases_profile_runtime_apply() {
        let mut state = M1State::default();
        set_perception_mode_in_state(
            &mut state,
            &SetPerceptionModeParams {
                mode: "pixel_only".to_owned(),
            },
        )
        .expect("manual mode parses");
        set_perception_mode_in_state(
            &mut state,
            &SetPerceptionModeParams {
                mode: "auto".to_owned(),
            },
        )
        .expect("auto mode parses");

        apply_profile_runtime_config_in_state(
            &mut state,
            &profile_with_mode(PerceptionMode::Hybrid),
        )
        .expect("profile config applies");

        assert_eq!(state.perception_mode, PerceptionMode::Hybrid);
        assert_eq!(state.manual_perception_mode, None);
    }

    #[test]
    fn read_text_resolves_focused_region_when_target_is_omitted() {
        let state = M1State {
            synthetic: Some(synthetic_notepad_input()),
            ..Default::default()
        };
        let focused = state
            .synthetic
            .as_ref()
            .and_then(|input| input.focused.as_ref())
            .expect("synthetic fixture has focused element")
            .bbox;

        let request = resolve_read_text_request(
            &state,
            &ReadTextParams {
                backend: OcrBackend::Auto,
                lang_hint: Some(" en-US ".to_owned()),
                ..ReadTextParams::default()
            },
        )
        .expect("focused fallback should resolve");

        assert_eq!(request.region, focused);
        assert_eq!(request.requested_backend, OcrBackend::Auto);
        assert_eq!(request.effective_backend, OcrBackend::Winrt);
        assert_eq!(request.lang(), "en-US");
        assert!(request.synthetic);
    }

    #[test]
    fn read_text_crnn_backend_fails_closed_until_provider_is_wired() {
        let state = M1State {
            synthetic: Some(synthetic_notepad_input()),
            ..Default::default()
        };

        let error = resolve_read_text_request(
            &state,
            &ReadTextParams {
                region: Some(Rect {
                    x: 1,
                    y: 2,
                    w: 80,
                    h: 24,
                }),
                backend: OcrBackend::Crnn,
                ..ReadTextParams::default()
            },
        )
        .expect_err("unwired CRNN backend must not silently fall through");

        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("code")),
            Some(&json!(error_codes::OCR_BACKEND_UNAVAILABLE))
        );
        assert!(error.message.contains("CRNN OCR backend"));
    }

    #[test]
    fn read_text_rejects_zero_sized_regions_before_ocr() {
        let state = M1State {
            synthetic: Some(synthetic_notepad_input()),
            ..Default::default()
        };

        for region in [
            Rect {
                x: 1,
                y: 2,
                w: 0,
                h: 24,
            },
            Rect {
                x: 1,
                y: 2,
                w: 80,
                h: 0,
            },
            Rect {
                x: 1,
                y: 2,
                w: -1,
                h: 24,
            },
            Rect {
                x: 1,
                y: 2,
                w: 80,
                h: -1,
            },
        ] {
            let error = resolve_read_text_request(
                &state,
                &ReadTextParams {
                    region: Some(region),
                    backend: OcrBackend::Winrt,
                    ..ReadTextParams::default()
                },
            )
            .expect_err("empty OCR regions must fail closed");
            assert_eq!(
                error.data.as_ref().and_then(|data| data.get("code")),
                Some(&json!(error_codes::OCR_NO_TEXT))
            );
        }
    }

    #[test]
    fn browser_ocr_words_upgrade_uia_only_to_queryable_ocr_nodes() {
        let mut input = chromium_ocr_input();
        let before_path = input.web_path;
        let before_len = input.elements.len();

        let added = apply_browser_ocr_words(
            &mut input,
            vec![
                TextRegion {
                    text: " Checkout ".to_owned(),
                    bbox: Rect {
                        x: 120,
                        y: 180,
                        w: 92,
                        h: 24,
                    },
                    confidence: 0.95,
                },
                TextRegion {
                    text: "now".to_owned(),
                    bbox: Rect {
                        x: 218,
                        y: 180,
                        w: 44,
                        h: 24,
                    },
                    confidence: 0.93,
                },
            ],
            8,
        );

        println!(
            "readback=browser_ocr edge=happy before_path:{before_path:?} before_elements:{before_len} after_path:{:?} after_elements:{} added:{added}",
            input.web_path,
            input.elements.len()
        );
        assert_eq!(added, 2);
        assert_eq!(input.web_path, Some(WebPerceptionPath::Ocr));
        assert_eq!(input.elements[0].name, "Checkout");
        assert_eq!(input.elements[0].role, "text");
        assert_eq!(
            input.elements[0].automation_id.as_deref(),
            Some("ocr:word:0")
        );
        assert!(
            input.elements[0]
                .element_id
                .parts()
                .expect("OCR element id parses")
                .runtime_id_hex
                .starts_with("0c0c")
        );
        assert_eq!(
            browser_ocr_rect_from_element_id(&input.elements[0].element_id),
            Some(Rect {
                x: 120,
                y: 180,
                w: 92,
                h: 24,
            })
        );
    }

    #[test]
    fn browser_ocr_words_keep_uia_only_when_ocr_has_no_usable_text() {
        let mut input = chromium_ocr_input();

        let added = apply_browser_ocr_words(
            &mut input,
            vec![
                TextRegion {
                    text: "   ".to_owned(),
                    bbox: Rect {
                        x: 1,
                        y: 2,
                        w: 30,
                        h: 12,
                    },
                    confidence: 0.5,
                },
                TextRegion {
                    text: "Hidden".to_owned(),
                    bbox: Rect {
                        x: 1,
                        y: 2,
                        w: 0,
                        h: 12,
                    },
                    confidence: 0.5,
                },
            ],
            8,
        );

        println!(
            "readback=browser_ocr edge=empty after_path:{:?} after_elements:{} added:{added}",
            input.web_path,
            input.elements.len()
        );
        assert_eq!(added, 0);
        assert_eq!(input.web_path, Some(WebPerceptionPath::UiaOnly));
        assert!(input.elements.is_empty());
    }

    #[test]
    fn browser_ocr_guard_only_allows_cdp_failures_on_uia_only_path() {
        let mut input = chromium_ocr_input();
        assert!(should_attempt_browser_ocr(&input));

        input.cdp = Some(CdpDiagnostics {
            process_name: "chrome.exe".to_owned(),
            status: CdpStatus::Ok,
            endpoint: Some("http://127.0.0.1:9222".to_owned()),
            checked_ports: vec![9222],
            checked_endpoints: vec!["http://127.0.0.1:9222".to_owned()],
            reason_code: None,
            detail: None,
            capabilities: Vec::new(),
            attached_node_count: None,
        });
        assert!(!should_attempt_browser_ocr(&input));

        input.cdp = Some(CdpDiagnostics::unreachable(
            "chrome.exe",
            error_codes::A11Y_CDP_UNREACHABLE,
        ));
        input.web_path = Some(WebPerceptionPath::Cdp);
        assert!(!should_attempt_browser_ocr(&input));
    }

    #[test]
    fn browser_ocr_skips_when_main_pane_uia_content_is_present() {
        let mut input = chromium_ocr_input();
        input.elements.push(AccessibleNode {
            element_id: synapse_core::element_id(0x2200, "0000002a00000042"),
            parent: None,
            name: "Force Renderer Complete Button".to_owned(),
            role: "button".to_owned(),
            automation_id: Some("probe-button".to_owned()),
            value: None,
            bbox: Rect {
                x: 420,
                y: 180,
                w: 320,
                h: 34,
            },
            enabled: true,
            focused: false,
            patterns: vec![synapse_core::UiaPattern::Invoke],
            children_count: 0,
            depth: 2,
        });

        println!(
            "readback=browser_ocr edge=main_pane_uia after_has_content:{} after_should_ocr:{}",
            has_chromium_main_pane_uia_content(&input),
            should_attempt_browser_ocr(&input)
        );
        assert!(has_chromium_main_pane_uia_content(&input));
        assert!(!should_attempt_browser_ocr(&input));
    }

    #[test]
    fn browser_ocr_runs_when_only_sidebar_uia_content_is_present() {
        let mut input = chromium_ocr_input();
        input.elements.push(AccessibleNode {
            element_id: synapse_core::element_id(0x2200, "0000002a00000043"),
            parent: None,
            name: "Navigation Item".to_owned(),
            role: "link".to_owned(),
            automation_id: Some("sidebar-link".to_owned()),
            value: None,
            bbox: Rect {
                x: 40,
                y: 180,
                w: 220,
                h: 34,
            },
            enabled: true,
            focused: false,
            patterns: vec![synapse_core::UiaPattern::Invoke],
            children_count: 0,
            depth: 2,
        });

        println!(
            "readback=browser_ocr edge=sidebar_only after_has_content:{} after_should_ocr:{}",
            has_chromium_main_pane_uia_content(&input),
            should_attempt_browser_ocr(&input)
        );
        assert!(!has_chromium_main_pane_uia_content(&input));
        assert!(should_attempt_browser_ocr(&input));
    }

    #[test]
    fn browser_content_tiles_skip_chrome_band_and_bound_tile_count() {
        let content = browser_content_region(Rect {
            x: 10,
            y: 20,
            w: 1200,
            h: 1600,
        })
        .expect("large browser window has content region");
        let tiles = browser_ocr_tiles(content);

        println!(
            "readback=browser_ocr edge=tiles content:{content:?} tile_count:{} first:{:?} last:{:?}",
            tiles.len(),
            tiles.first(),
            tiles.last()
        );
        assert_eq!(content.y, 116);
        assert_eq!(content.h, 1504);
        assert_eq!(tiles.len(), 3);
        assert_eq!(tiles[0].h, BROWSER_OCR_TILE_HEIGHT_PX);
        assert_eq!(tiles[2].h, 304);
        assert!(
            browser_content_region(Rect {
                x: 0,
                y: 0,
                w: 10,
                h: 0,
            })
            .is_none()
        );
    }

    fn chromium_ocr_input() -> ObservationInput {
        let mut input = ObservationInput::new(ForegroundContext {
            hwnd: 0x2200,
            pid: 7777,
            process_name: "chrome.exe".to_owned(),
            process_path: "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe".to_owned(),
            window_title: "Example - Google Chrome".to_owned(),
            window_bounds: Rect {
                x: 0,
                y: 0,
                w: 1280,
                h: 900,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: Some("chrome".to_owned()),
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        });
        input.capture_status = SensorStatus::Healthy;
        input.cdp = Some(CdpDiagnostics::unreachable(
            "chrome.exe",
            error_codes::A11Y_CDP_UNREACHABLE,
        ));
        input.web_path = Some(WebPerceptionPath::UiaOnly);
        input
    }

    fn profile_with_mode(mode: PerceptionMode) -> Profile {
        Profile {
            id: "test-profile".to_owned(),
            label: "Test Profile".to_owned(),
            version: "2".to_owned(),
            use_scope: ProfileUseScope::OperatorOwnedTest,
            matches: vec![ProfileMatch {
                exe: Some("test.exe".to_owned()),
                title_regex: None,
                steam_appid: None,
                window_class: None,
                process_args: Vec::new(),
            }],
            mode,
            capture: ProfileCapture {
                target: ProfileCaptureTarget::ForegroundWindow,
                min_update_interval_ms: 50,
                cursor_visible: true,
            },
            detection: ProfileDetection {
                model_id: None,
                classes_of_interest: Vec::new(),
                confidence_threshold: 0.5,
                max_detections: 32,
            },
            ocr: ProfileOcr {
                default_backend: OcrBackend::Auto,
                regions: Vec::new(),
                parser_config: BTreeMap::new(),
            },
            hud: Vec::new(),
            keymap: BTreeMap::new(),
            backends: ProfileBackends {
                default: Backend::Auto,
                keyboard_default: Backend::Auto,
                mouse_default: Backend::Auto,
                pad_default: Backend::Auto,
            },
            metadata: BTreeMap::new(),
            event_extensions: Vec::new(),
        }
    }
}
