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
pub use ocr::read_text_request_from_bgra;
pub use ocr::{ResolvedReadTextRequest, read_text_request_uncached, resolve_read_text_request};
use search::{element_match, entity_match};
pub use sources::{FsRecentTracker, populate_clipboard_summary, populate_fs_recent};
use sources::{platform_input, synthetic_notepad_input, window_input_from_hwnd};

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
    if !capture_config_without_generation_eq(&state.active_capture_config, &active_capture) {
        state.capture_generation = state.capture_generation.saturating_add(1);
        active_capture.generation = state.capture_generation;
    } else {
        active_capture.generation = state.active_capture_config.generation;
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
        Backend, ProfileBackends, ProfileDetection, ProfileMatch, ProfileOcr, ProfileUseScope,
    };

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
