//! Suggestion engine (#858, epic #832/#828).
//!
//! The decision layer between intent detection (#854/#855) and the human-facing
//! approval/assist surface (#833). Given the routines the operator appears to be
//! executing right now (the same engine `intent_current` uses), it decides
//! whether to surface a suggestion — and, crucially, when NOT to. The
//! anti-"Clippy" gates are the product, not polish:
//!
//! 1. confidence threshold (default high)
//! 2. feedback suppression / decline cooldown (#856)
//! 3. quiet hours
//! 4. dedup: at most one LIVE suggestion per routine
//! 5. per-routine frequency cap (one per routine per window)
//! 6. global frequency cap (N per rolling window)
//! 7. disabled/archived routines never surface
//!
//! Live suggestions terminate by timeout (→ `ignored_timeout` feedback) or by
//! the routine dropping out of the live intent set (→ `abandoned` feedback),
//! closing the loop back into #856. Accept/decline come from the execution /
//! approval path (#860/#833) and are out of this module's scope.
//!
//! Truth lives in `CF_KV` under `suggestion/v1/`, never daemon memory: a daemon
//! restart re-derives every cap and dedup decision from the persisted rows.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{Local, TimeZone, Timelike};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use synapse_core::error_codes;
use synapse_core::intent::IntentCandidate;
use synapse_core::types::{RoutineFeedbackOutcome, RoutineLifecycle};
use synapse_storage::{Db, cf, decode_json, encode_json};

use crate::m1::mcp_error;

use super::episodes::now_ts_ns;
use super::intent::{IntentCurrentParams, current_intents};
use super::permissions::{Permission, RequiredPermissions, required};
use super::routines::{
    RoutineFeedbackParams, feedback_suppressed, load_state_row, record_routine_feedback,
};

/// `CF_KV` key prefix for suggestion rows.
const SUGGESTION_PREFIX: &str = "suggestion/v1/";
/// Schema version for [`SuggestionRecord`].
const SUGGESTION_RECORD_VERSION: u32 = 1;
/// The engine actor recorded on feedback it generates.
const SUGGESTION_ACTOR: &str = "suggestion-engine";

const ENGINE_VERSION_DEFAULTS: &str = "see SuggestionConfig::from_env";

/// Engine knobs. Defaults are deliberately conservative (anti-Clippy);
/// every one is overridable by env so the gates are FSV-testable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SuggestionConfig {
    /// Minimum intent confidence to surface (default 0.6).
    pub min_confidence: f64,
    /// How long a live suggestion stays live before timing out (default 600s).
    pub expiry_secs: u64,
    /// Max suggestions created per rolling global window (default 5).
    pub global_max: u32,
    /// The global rolling window (default 3600s).
    pub global_window_secs: u64,
    /// Minimum spacing between suggestions for the SAME routine (default 4h).
    pub per_routine_window_secs: u64,
    /// Optional quiet-hours window as local minutes-of-day `[start, end)`.
    /// Wraps past midnight when start > end. `None` disables quiet hours.
    pub quiet_hours: Option<(u32, u32)>,
}

impl SuggestionConfig {
    fn env_f64(name: &str, default: f64) -> f64 {
        std::env::var(name)
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(default)
    }
    fn env_u64(name: &str, default: u64) -> u64 {
        std::env::var(name)
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(default)
    }
    fn env_u32(name: &str, default: u32) -> u32 {
        std::env::var(name)
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(default)
    }

    #[must_use]
    pub fn from_env() -> Self {
        let quiet_start = std::env::var("SYNAPSE_SUGGEST_QUIET_START_MIN")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok());
        let quiet_end = std::env::var("SYNAPSE_SUGGEST_QUIET_END_MIN")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok());
        let quiet_hours = match (quiet_start, quiet_end) {
            (Some(start), Some(end)) if start < 1440 && end < 1440 => Some((start, end)),
            _ => None,
        };
        Self {
            min_confidence: Self::env_f64("SYNAPSE_SUGGEST_MIN_CONFIDENCE", 0.6),
            expiry_secs: Self::env_u64("SYNAPSE_SUGGEST_EXPIRY_SECS", 600),
            global_max: Self::env_u32("SYNAPSE_SUGGEST_GLOBAL_MAX", 5),
            global_window_secs: Self::env_u64("SYNAPSE_SUGGEST_GLOBAL_WINDOW_SECS", 3_600),
            per_routine_window_secs: Self::env_u64(
                "SYNAPSE_SUGGEST_PER_ROUTINE_WINDOW_SECS",
                14_400,
            ),
            quiet_hours,
        }
    }
}

/// Lifecycle of a surfaced suggestion.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStatus {
    /// Surfaced and awaiting the operator.
    Live,
    /// Operator accepted (set by the execution/approval path, #860/#833).
    Accepted,
    /// Operator declined (set by the approval path).
    Declined,
    /// Timed out unanswered.
    Expired,
    /// The routine dropped out of the live intent set before resolution.
    Abandoned,
}

/// One surfaced suggestion, persisted in `CF_KV`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestionRecord {
    pub record_version: u32,
    pub suggestion_id: String,
    pub routine_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub created_ts_ns: u64,
    pub expiry_ts_ns: u64,
    pub status: SuggestionStatus,
    /// Intent confidence at creation (the value the threshold gate saw).
    pub confidence: f64,
    pub matched_prefix_len: u32,
    pub total_steps: u32,
    pub remaining_step_count: u32,
    /// Compiled plan reference (filled by #859 once a plan exists).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_plan_ref: Option<String>,
    /// When the suggestion left `Live` (expiry/abandon/accept/decline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_ts_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_note: Option<String>,
}

/// Why a candidate did NOT surface (or that it did). Ordered by the gate's
/// short-circuit precedence.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateOutcome {
    Surface,
    DisabledRoutine,
    BelowThreshold,
    SuppressedCooldown,
    QuietHours,
    DuplicateLive,
    PerRoutineCap,
    GlobalCap,
}

/// Pre-computed aggregates over existing suggestions, so the gate stays a pure
/// function (unit-testable without storage).
#[derive(Clone, Debug, Default)]
pub struct SuggestionAggregates {
    pub live_routines: BTreeSet<String>,
    /// routine_id → most recent created_ts_ns across ALL statuses.
    pub last_created_by_routine: BTreeMap<String, u64>,
    /// created_ts_ns of every suggestion (any status), for the global window.
    pub created_ts: Vec<u64>,
}

/// Local minute-of-day for `now_ns`, or `None` if the clock is out of range.
#[must_use]
pub fn local_minute_of_day(now_ns: u64) -> Option<u32> {
    let secs = i64::try_from(now_ns / 1_000_000_000).ok()?;
    match Local.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(dt) => Some(dt.hour() * 60 + dt.minute()),
        _ => None,
    }
}

#[must_use]
fn in_quiet_hours(minute: u32, quiet: Option<(u32, u32)>) -> bool {
    match quiet {
        None => false,
        Some((start, end)) if start <= end => minute >= start && minute < end,
        // Wrapping window (e.g. 22:00–07:00).
        Some((start, end)) => minute >= start || minute < end,
    }
}

/// Pure gate: decide whether ONE candidate should surface. `suppressed` is the
/// #856 feedback cooldown verdict; the aggregates supply dedup/cap context.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn gate_decision(
    routine_id: &str,
    confidence: f64,
    lifecycle: RoutineLifecycle,
    suppressed: bool,
    now_ns: u64,
    now_minute: Option<u32>,
    aggregates: &SuggestionAggregates,
    config: &SuggestionConfig,
) -> GateOutcome {
    if matches!(
        lifecycle,
        RoutineLifecycle::Disabled | RoutineLifecycle::Archived
    ) {
        return GateOutcome::DisabledRoutine;
    }
    if confidence < config.min_confidence {
        return GateOutcome::BelowThreshold;
    }
    if suppressed {
        return GateOutcome::SuppressedCooldown;
    }
    if let Some(minute) = now_minute {
        if in_quiet_hours(minute, config.quiet_hours) {
            return GateOutcome::QuietHours;
        }
    }
    if aggregates.live_routines.contains(routine_id) {
        return GateOutcome::DuplicateLive;
    }
    if let Some(last) = aggregates.last_created_by_routine.get(routine_id) {
        if now_ns.saturating_sub(*last) < config.per_routine_window_secs.saturating_mul(1_000_000_000)
        {
            return GateOutcome::PerRoutineCap;
        }
    }
    let window_floor = now_ns.saturating_sub(config.global_window_secs.saturating_mul(1_000_000_000));
    let global_count = aggregates
        .created_ts
        .iter()
        .filter(|ts| **ts >= window_floor)
        .count();
    if u32::try_from(global_count).unwrap_or(u32::MAX) >= config.global_max {
        return GateOutcome::GlobalCap;
    }
    GateOutcome::Surface
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestionTickParams {
    /// Evaluate as of this instant (replay/test). Defaults to now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now_ts_ns: Option<u64>,
    /// Recent-activity lookback handed to the intent matcher (default 6h).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lookback_hours: Option<u32>,
    /// Compute the decision for every candidate but persist nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// One per-candidate gate decision, echoed for auditability.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GateDecisionRow {
    pub routine_id: String,
    pub confidence: f64,
    pub outcome: GateOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestionTickResponse {
    pub now_ts_ns: u64,
    pub dry_run: bool,
    pub candidates_evaluated: u32,
    pub created: Vec<String>,
    pub expired: Vec<String>,
    pub abandoned: Vec<String>,
    /// Every candidate's gate decision (created or suppressed-with-reason).
    pub decisions: Vec<GateDecisionRow>,
    pub config: SuggestionConfigEcho,
}

/// Serializable echo of the active config (the opaque struct is not `JsonSchema`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestionConfigEcho {
    pub min_confidence: f64,
    pub expiry_secs: u64,
    pub global_max: u32,
    pub global_window_secs: u64,
    pub per_routine_window_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_hours: Option<[u32; 2]>,
}

impl From<SuggestionConfig> for SuggestionConfigEcho {
    fn from(c: SuggestionConfig) -> Self {
        Self {
            min_confidence: c.min_confidence,
            expiry_secs: c.expiry_secs,
            global_max: c.global_max,
            global_window_secs: c.global_window_secs,
            per_routine_window_secs: c.per_routine_window_secs,
            quiet_hours: c.quiet_hours.map(|q| [q.0, q.1]),
        }
    }
}

pub fn required_permissions_tick(_params: &SuggestionTickParams) -> RequiredPermissions {
    required([Permission::ReadStorage, Permission::WriteStorage])
}

fn storage_error(error: impl std::fmt::Display) -> ErrorData {
    mcp_error(
        error_codes::STORAGE_READ_FAILED,
        format!("suggestion engine storage failure: {error}"),
    )
}

fn suggestion_key(routine_id: &str, created_ts_ns: u64) -> Vec<u8> {
    format!("{SUGGESTION_PREFIX}{routine_id}/{created_ts_ns:020}").into_bytes()
}

/// Loads every suggestion row, newest decode first is irrelevant (callers
/// aggregate). Loud on undecodable rows.
fn load_all_suggestions(db: &Arc<Db>) -> Result<Vec<(Vec<u8>, SuggestionRecord)>, ErrorData> {
    let rows = db
        .scan_cf_prefix(cf::CF_KV, SUGGESTION_PREFIX.as_bytes())
        .map_err(storage_error)?;
    let mut out = Vec::with_capacity(rows.len());
    for (key, value) in rows {
        let record: SuggestionRecord = decode_json(&value).map_err(|error| {
            mcp_error(
                error_codes::STORAGE_CORRUPTED,
                format!(
                    "SUGGESTION_ROW_DECODE_FAILED in CF_KV at {}: {error}",
                    String::from_utf8_lossy(&key)
                ),
            )
        })?;
        out.push((key, record));
    }
    Ok(out)
}

fn write_suggestion(db: &Arc<Db>, record: &SuggestionRecord) -> Result<(), ErrorData> {
    let key = suggestion_key(&record.routine_id, record.created_ts_ns);
    let value = encode_json(record).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_WRITE_FAILED,
            format!("failed to encode suggestion {}: {error}", record.suggestion_id),
        )
    })?;
    db.put_batch_pressure_bypass(cf::CF_KV, [(key.clone(), value)])
        .map_err(|error| {
            mcp_error(
                error_codes::STORAGE_WRITE_FAILED,
                format!("failed to persist suggestion {}: {error}", record.suggestion_id),
            )
        })?;
    // Read-your-write against the physical row.
    let rows = db.scan_cf_prefix(cf::CF_KV, &key).map_err(storage_error)?;
    match rows.first() {
        Some((_, value)) => {
            let readback: SuggestionRecord = decode_json(value).map_err(storage_error)?;
            if &readback != record {
                return Err(mcp_error(
                    error_codes::STORAGE_CORRUPTED,
                    format!(
                        "SUGGESTION_READBACK_MISMATCH for {}: persisted row != value just written",
                        record.suggestion_id
                    ),
                ));
            }
            Ok(())
        }
        None => Err(mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!(
                "SUGGESTION_READBACK_MISSING: row for {} vanished immediately after write",
                record.suggestion_id
            ),
        )),
    }
}

fn record_terminal_feedback(
    db: &Arc<Db>,
    routine_id: &str,
    outcome: RoutineFeedbackOutcome,
    now_ns: u64,
    note: &str,
) -> Result<(), ErrorData> {
    let params = RoutineFeedbackParams {
        routine_id: routine_id.to_owned(),
        outcome,
        note: Some(note.to_owned()),
        now_ts_ns: Some(now_ns),
    };
    record_routine_feedback(db, &params, SUGGESTION_ACTOR).map(|_| ())
}

/// One engine pass: expire timed-out suggestions, abandon ones whose routine
/// left the live set, then create suggestions for fresh candidates that pass
/// every gate. Each terminal transition records #856 feedback.
pub fn suggestion_tick(
    db: &Arc<Db>,
    params: &SuggestionTickParams,
) -> Result<SuggestionTickResponse, ErrorData> {
    let _ = ENGINE_VERSION_DEFAULTS;
    let now = params.now_ts_ns.unwrap_or_else(now_ts_ns);
    let config = SuggestionConfig::from_env();

    if !db.pressure_permits_write(cf::CF_KV) && !params.dry_run {
        return Err(mcp_error(
            error_codes::STORAGE_WRITE_FAILED,
            format!(
                "suggestion_tick refused under disk pressure: pressure_level={:?}",
                db.pressure_level()
            ),
        ));
    }

    // Current live intents (the detection signal). Floor at the engine
    // threshold so the candidate set is exactly the surfacing-eligible ones.
    let intent = current_intents(
        db,
        &IntentCurrentParams {
            now_ts_ns: Some(now),
            lookback_hours: params.lookback_hours,
            min_confidence: Some(0.0),
            max_candidates: Some(50),
            include_agent_activity: false,
        },
    )?;
    let candidate_routines: BTreeSet<String> = intent
        .candidates
        .iter()
        .map(|c| c.routine_id.clone())
        .collect();

    let mut suggestions = load_all_suggestions(db)?;
    let mut expired = Vec::new();
    let mut abandoned = Vec::new();

    // --- Expire / abandon pass over live suggestions ---
    for (_key, record) in &mut suggestions {
        if record.status != SuggestionStatus::Live {
            continue;
        }
        if now >= record.expiry_ts_ns {
            record.status = SuggestionStatus::Expired;
            record.resolved_ts_ns = Some(now);
            record.resolution_note = Some("timed out unanswered".to_owned());
            if !params.dry_run {
                write_suggestion(db, record)?;
                record_terminal_feedback(
                    db,
                    &record.routine_id,
                    RoutineFeedbackOutcome::IgnoredTimeout,
                    now,
                    "suggestion expired (timeout)",
                )?;
            }
            expired.push(record.suggestion_id.clone());
        } else if !candidate_routines.contains(&record.routine_id) {
            record.status = SuggestionStatus::Abandoned;
            record.resolved_ts_ns = Some(now);
            record.resolution_note = Some("routine left the live intent set".to_owned());
            if !params.dry_run {
                write_suggestion(db, record)?;
                record_terminal_feedback(
                    db,
                    &record.routine_id,
                    RoutineFeedbackOutcome::Abandoned,
                    now,
                    "suggestion abandoned (intent dropped)",
                )?;
            }
            abandoned.push(record.suggestion_id.clone());
        }
    }

    // --- Aggregates AFTER expiry/abandon (so a just-expired routine is no
    // longer "live" for dedup, and caps count history honestly). Mutated as the
    // creation pass adds suggestions, so a second candidate respects the caps. ---
    let mut live = build_aggregates(&suggestions);

    // --- Creation pass ---
    let mut created = Vec::new();
    let mut decisions = Vec::new();
    let now_minute = local_minute_of_day(now);
    for candidate in &intent.candidates {
        let suppressed = is_routine_suppressed(db, &candidate.routine_id, now)?;
        let outcome = gate_decision(
            &candidate.routine_id,
            candidate.confidence,
            candidate.lifecycle,
            suppressed,
            now,
            now_minute,
            &live,
            &config,
        );
        let mut created_id = None;
        if outcome == GateOutcome::Surface && !params.dry_run {
            let record = build_suggestion(candidate, now, &config);
            write_suggestion(db, &record)?;
            // Update in-tick aggregates so a second candidate respects the caps.
            live.live_routines.insert(record.routine_id.clone());
            live.last_created_by_routine
                .insert(record.routine_id.clone(), record.created_ts_ns);
            live.created_ts.push(record.created_ts_ns);
            created.push(record.suggestion_id.clone());
            created_id = Some(record.suggestion_id.clone());
        } else if outcome == GateOutcome::Surface && params.dry_run {
            created_id = Some(format!("(dry-run){}", candidate.routine_id));
        }
        decisions.push(GateDecisionRow {
            routine_id: candidate.routine_id.clone(),
            confidence: candidate.confidence,
            outcome,
            suggestion_id: created_id,
        });
    }

    Ok(SuggestionTickResponse {
        now_ts_ns: now,
        dry_run: params.dry_run,
        candidates_evaluated: u32::try_from(intent.candidates.len()).unwrap_or(u32::MAX),
        created,
        expired,
        abandoned,
        decisions,
        config: config.into(),
    })
}

fn build_aggregates(suggestions: &[(Vec<u8>, SuggestionRecord)]) -> SuggestionAggregates {
    let mut agg = SuggestionAggregates::default();
    for (_key, record) in suggestions {
        if record.status == SuggestionStatus::Live {
            agg.live_routines.insert(record.routine_id.clone());
        }
        let entry = agg
            .last_created_by_routine
            .entry(record.routine_id.clone())
            .or_insert(0);
        *entry = (*entry).max(record.created_ts_ns);
        agg.created_ts.push(record.created_ts_ns);
    }
    agg
}

fn build_suggestion(
    candidate: &IntentCandidate,
    now: u64,
    config: &SuggestionConfig,
) -> SuggestionRecord {
    SuggestionRecord {
        record_version: SUGGESTION_RECORD_VERSION,
        suggestion_id: format!("sg1-{}-{now:020}", candidate.routine_id),
        routine_id: candidate.routine_id.clone(),
        label: candidate.label.clone(),
        created_ts_ns: now,
        expiry_ts_ns: now.saturating_add(config.expiry_secs.saturating_mul(1_000_000_000)),
        status: SuggestionStatus::Live,
        confidence: candidate.confidence,
        matched_prefix_len: u32::try_from(candidate.matched_prefix_len).unwrap_or(u32::MAX),
        total_steps: u32::try_from(candidate.total_steps).unwrap_or(u32::MAX),
        remaining_step_count: u32::try_from(candidate.remaining_steps.len()).unwrap_or(u32::MAX),
        proposed_plan_ref: None,
        resolved_ts_ns: None,
        resolution_note: None,
    }
}

fn is_routine_suppressed(db: &Arc<Db>, routine_id: &str, now: u64) -> Result<bool, ErrorData> {
    Ok(match load_state_row(db, routine_id)? {
        Some(state) => feedback_suppressed(&state, now),
        None => false,
    })
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestionListParams {
    /// Filter by status (live/accepted/declined/expired/abandoned).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<SuggestionStatus>,
    /// Filter to one routine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routine_id: Option<String>,
    /// Max rows (default 100, max 1000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestionListResponse {
    pub suggestions: Vec<SuggestionRecord>,
    pub total_rows: u64,
    pub returned: u64,
}

pub fn required_permissions_list(_params: &SuggestionListParams) -> RequiredPermissions {
    required([Permission::ReadStorage])
}

pub fn list_suggestions(
    db: &Arc<Db>,
    params: &SuggestionListParams,
) -> Result<SuggestionListResponse, ErrorData> {
    let limit = params.limit.unwrap_or(100).min(1000) as usize;
    let all = load_all_suggestions(db)?;
    let total_rows = all.len() as u64;
    let mut filtered: Vec<SuggestionRecord> = all
        .into_iter()
        .map(|(_key, record)| record)
        .filter(|record| params.status.is_none_or(|status| record.status == status))
        .filter(|record| {
            params
                .routine_id
                .as_ref()
                .is_none_or(|routine_id| &record.routine_id == routine_id)
        })
        .collect();
    // Newest first.
    filtered.sort_by_key(|record| std::cmp::Reverse(record.created_ts_ns));
    filtered.truncate(limit);
    let returned = filtered.len() as u64;
    Ok(SuggestionListResponse {
        suggestions: filtered,
        total_rows,
        returned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> SuggestionConfig {
        SuggestionConfig {
            min_confidence: 0.6,
            expiry_secs: 600,
            global_max: 3,
            global_window_secs: 3_600,
            per_routine_window_secs: 14_400,
            quiet_hours: None,
        }
    }
    const T: u64 = 1_000_000_000_000_000_000;

    #[test]
    fn gate_blocks_below_threshold_disabled_and_suppressed() {
        let agg = SuggestionAggregates::default();
        let c = config();
        // Below threshold.
        assert_eq!(
            gate_decision("rt1-a", 0.59, RoutineLifecycle::Confirmed, false, T, Some(600), &agg, &c),
            GateOutcome::BelowThreshold
        );
        // Disabled routine never surfaces, even at high confidence.
        assert_eq!(
            gate_decision("rt1-a", 0.99, RoutineLifecycle::Disabled, false, T, Some(600), &agg, &c),
            GateOutcome::DisabledRoutine
        );
        // Feedback cooldown suppresses.
        assert_eq!(
            gate_decision("rt1-a", 0.99, RoutineLifecycle::Confirmed, true, T, Some(600), &agg, &c),
            GateOutcome::SuppressedCooldown
        );
        // Clean pass.
        assert_eq!(
            gate_decision("rt1-a", 0.99, RoutineLifecycle::Confirmed, false, T, Some(600), &agg, &c),
            GateOutcome::Surface
        );
    }

    #[test]
    fn gate_enforces_dedup_per_routine_and_global_caps() {
        let c = config();
        // One live suggestion for the routine -> duplicate.
        let mut agg = SuggestionAggregates::default();
        agg.live_routines.insert("rt1-a".to_owned());
        assert_eq!(
            gate_decision("rt1-a", 0.99, RoutineLifecycle::Confirmed, false, T, None, &agg, &c),
            GateOutcome::DuplicateLive
        );
        // Recent (non-live) suggestion for the routine within the window -> cap.
        let mut agg = SuggestionAggregates::default();
        agg.last_created_by_routine
            .insert("rt1-a".to_owned(), T - 1_000_000_000); // 1s ago, window 4h
        assert_eq!(
            gate_decision("rt1-a", 0.99, RoutineLifecycle::Confirmed, false, T, None, &agg, &c),
            GateOutcome::PerRoutineCap
        );
        // Outside the per-routine window -> allowed (only global applies).
        let mut agg = SuggestionAggregates::default();
        agg.last_created_by_routine
            .insert("rt1-a".to_owned(), T - 20_000 * 1_000_000_000); // >4h ago
        assert_eq!(
            gate_decision("rt1-a", 0.99, RoutineLifecycle::Confirmed, false, T, None, &agg, &c),
            GateOutcome::Surface
        );
        // Global cap: 3 created within the window -> next is blocked.
        let mut agg = SuggestionAggregates::default();
        agg.created_ts = vec![T - 1, T - 2, T - 3];
        assert_eq!(
            gate_decision("rt1-b", 0.99, RoutineLifecycle::Confirmed, false, T, None, &agg, &c),
            GateOutcome::GlobalCap
        );
        // Old creations fall out of the window -> allowed again.
        let mut agg = SuggestionAggregates::default();
        agg.created_ts = vec![T - 5_000 * 1_000_000_000; 3];
        assert_eq!(
            gate_decision("rt1-b", 0.99, RoutineLifecycle::Confirmed, false, T, None, &agg, &c),
            GateOutcome::Surface
        );
    }

    #[test]
    fn quiet_hours_block_including_wraparound() {
        let mut c = config();
        c.quiet_hours = Some((1320, 420)); // 22:00–07:00 wrap
        let agg = SuggestionAggregates::default();
        // 23:00 -> quiet.
        assert_eq!(
            gate_decision("rt1-a", 0.99, RoutineLifecycle::Confirmed, false, T, Some(1380), &agg, &c),
            GateOutcome::QuietHours
        );
        // 03:00 -> quiet (wrap).
        assert_eq!(
            gate_decision("rt1-a", 0.99, RoutineLifecycle::Confirmed, false, T, Some(180), &agg, &c),
            GateOutcome::QuietHours
        );
        // 12:00 -> awake.
        assert_eq!(
            gate_decision("rt1-a", 0.99, RoutineLifecycle::Confirmed, false, T, Some(720), &agg, &c),
            GateOutcome::Surface
        );
    }
}
