//! `routine_mine` MCP tool (#848, epic #830).
//!
//! Runs the deterministic routine mining engine
//! ([`synapse_core::routines::mine_routines`]) over `CF_EPISODES` and
//! replaces `CF_ROUTINES` with the result in one atomic flushed batch.
//! Routines are derived state: the store always holds exactly one mining
//! run's complete output, so re-mining is idempotent by construction.
//!
//! The same entry point serves the on-demand MCP tool and the periodic
//! in-daemon batch job ([`super::routine_miner_job`]); a process-wide mining
//! lock serializes the two so concurrent replace-alls can never interleave.
//!
//! Failure policy: disk pressure refusal, undecodable derived rows, scan
//! budget exhaustion, and engine errors are loud and structured. The tool
//! never replaces rows it could not fully re-derive.

use std::sync::{Arc, Mutex};

use chrono::{Datelike, Local, TimeZone};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use synapse_core::error_codes;
use synapse_core::routines::{MiningDay, RoutineMiningConfig, mine_routines};
use synapse_core::types::RoutineRecord;
use synapse_storage::{Db, cf, encode_json, routines as routine_codec};

use crate::m1::mcp_error;

use super::episodes::{
    decode_episode_row, hex_encode, key_after, local_day_start, next_local_day_start, now_ts_ns,
};
use super::{
    M3ToolStub,
    permissions::{Permission, RequiredPermissions, required},
};

/// Maximum `CF_EPISODES`/`CF_ROUTINES` rows scanned per call. Exceeding it
/// is a structured error, never a partial mine: routine support counts
/// derived from a truncated episode scan would be silently wrong.
pub const MAX_SCAN_ROWS_PER_CALL: usize = 200_000;
/// Chunk size for bounded storage reads inside one call.
const SCAN_CHUNK_ROWS: usize = 4_096;
/// Upper bound for the `max_pattern_len` parameter.
pub const MAX_PATTERN_LEN_LIMIT: u32 = 12;
/// Upper bound for the `min_support_days` parameter (the mining window is
/// at most the 90-day episode retention horizon).
pub const MIN_SUPPORT_DAYS_LIMIT: u32 = 92;

/// Process-wide mining serialization: the on-demand tool and the periodic
/// job must never run replace-all concurrently.
static MINE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutineMineParams {
    /// Inclusive lower bound; snapped DOWN to its local midnight. Defaults
    /// to the first episode row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ts_ns: Option<u64>,
    /// Exclusive upper bound; snapped UP to the next local midnight.
    /// Defaults to now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ts_ns: Option<u64>,
    /// Distinct-day support floor (default 3, max 92).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_support_days: Option<u32>,
    /// Episodes shorter than this are excluded (default 60000 ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_episode_duration_ms: Option<u64>,
    /// Longest mined template in steps (default 6, max 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pattern_len: Option<u32>,
    /// Mine agent-actor episodes too (default false: human routines only).
    #[serde(default)]
    pub include_agent_activity: bool,
    /// Compute everything but mutate nothing.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutineMineResponse {
    /// Effective day-snapped range this call mined.
    pub range_start_ns: u64,
    pub range_end_ns: u64,
    /// `CF_EPISODES` rows examined.
    pub scanned_episode_rows: u64,
    /// Episodes fed to the engine across all days.
    pub considered_episodes: u64,
    /// Episodes that survived the eligibility filter.
    pub eligible_episodes: u64,
    pub filtered_agent_episodes: u64,
    pub filtered_short_episodes: u64,
    pub filtered_no_app_episodes: u64,
    /// Days in the window with at least one eligible episode.
    pub active_days: u32,
    /// Distinct candidate patterns tracked.
    pub candidates_evaluated: u64,
    /// New patterns ignored after the candidate cap was reached.
    pub candidates_truncated: u64,
    /// Occurrences ignored after a pattern hit its per-day cap.
    pub occurrences_skipped_over_cap: u64,
    pub clusters_rejected_low_support: u64,
    pub clusters_rejected_dispersed: u64,
    pub clusters_rejected_low_confidence: u64,
    pub candidates_rejected_as_subpattern: u64,
    pub routines_dropped_over_cap: u64,
    /// Rows written to `CF_ROUTINES` (0 on dry runs).
    pub routines_written: u64,
    /// Stale rows deleted from `CF_ROUTINES` (0 on dry runs).
    pub routines_deleted: u64,
    pub dry_run: bool,
    /// The mined routines, strongest first (full persisted records).
    pub routines: Vec<RoutineRecord>,
}

#[must_use]
pub const fn routine_mine() -> M3ToolStub {
    M3ToolStub::new("routine_mine")
}

#[must_use]
pub fn required_permissions(_params: &RoutineMineParams) -> RequiredPermissions {
    required([Permission::ReadStorage, Permission::WriteStorage])
}

fn invalid(detail: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, detail.into())
}

fn internal(detail: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_INTERNAL_ERROR, detail.into())
}

fn build_config(params: &RoutineMineParams) -> Result<RoutineMiningConfig, ErrorData> {
    let mut config = RoutineMiningConfig::default();
    if let Some(min_support_days) = params.min_support_days {
        if min_support_days == 0 || min_support_days > MIN_SUPPORT_DAYS_LIMIT {
            return Err(invalid(format!(
                "routine_mine min_support_days must be between 1 and {MIN_SUPPORT_DAYS_LIMIT}; \
                 got {min_support_days}"
            )));
        }
        config.min_support_days = min_support_days;
    }
    if let Some(max_pattern_len) = params.max_pattern_len {
        if max_pattern_len == 0 || max_pattern_len > MAX_PATTERN_LEN_LIMIT {
            return Err(invalid(format!(
                "routine_mine max_pattern_len must be between 1 and {MAX_PATTERN_LEN_LIMIT}; \
                 got {max_pattern_len}"
            )));
        }
        config.max_pattern_len = max_pattern_len as usize;
    }
    if let Some(min_episode_duration_ms) = params.min_episode_duration_ms {
        if min_episode_duration_ms > 86_400_000 {
            return Err(invalid(format!(
                "routine_mine min_episode_duration_ms must be at most one day (86400000); \
                 got {min_episode_duration_ms}"
            )));
        }
        config.min_episode_duration_ns = min_episode_duration_ms.saturating_mul(1_000_000);
    }
    config.include_agent_activity = params.include_agent_activity;
    Ok(config)
}

/// First decodable `CF_EPISODES` key timestamp, if any.
fn first_episode_ts(db: &Db, scanned_rows: &mut u64) -> Result<Option<u64>, ErrorData> {
    let (rows, _more) = db
        .scan_cf_from(cf::CF_EPISODES, &[], 1)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    let Some((key, value)) = rows.first() else {
        return Ok(None);
    };
    *scanned_rows += 1;
    let (key_ts_ns, _ordinal, _record) = decode_episode_row(key, value)?;
    Ok(Some(key_ts_ns))
}

/// 0 = Monday … 6 = Sunday for a local-midnight timestamp.
fn weekday_of_day_start(day_start_ns: u64) -> Result<u8, ErrorData> {
    let ts = i64::try_from(day_start_ns).map_err(|_e| {
        internal(format!(
            "day_start_ns {day_start_ns} exceeds the representable range"
        ))
    })?;
    let weekday = Local.timestamp_nanos(ts).weekday().num_days_from_monday();
    u8::try_from(weekday).map_err(|_e| internal("weekday outside 0..=6"))
}

/// Collects episodes in `[range_start_ns, range_end_ns)` grouped into local
/// mining days, in chronological order. Fails loudly on undecodable derived
/// rows and on scan budget exhaustion — never a partial mine.
fn mining_days(
    db: &Db,
    range_start_ns: u64,
    range_end_ns: u64,
    scanned_rows: &mut u64,
) -> Result<Vec<MiningDay>, ErrorData> {
    let mut days: Vec<MiningDay> = Vec::new();
    let mut current_day_start: Option<u64> = None;
    let mut start = synapse_storage::episodes::episode_scan_start(range_start_ns);
    'scan: loop {
        if usize::try_from(*scanned_rows).unwrap_or(usize::MAX) >= MAX_SCAN_ROWS_PER_CALL {
            return Err(internal(format!(
                "ROUTINE_SCAN_BUDGET_EXHAUSTED after {MAX_SCAN_ROWS_PER_CALL} CF_EPISODES rows; \
                 pass a narrower start_ts_ns/end_ts_ns range — mining over a truncated scan \
                 would fabricate support counts"
            )));
        }
        let (rows, more) = db
            .scan_cf_from(cf::CF_EPISODES, &start, SCAN_CHUNK_ROWS)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        if rows.is_empty() {
            break;
        }
        for (key, value) in &rows {
            *scanned_rows += 1;
            let (key_ts_ns, _ordinal, record) = decode_episode_row(key, value)?;
            if key_ts_ns >= range_end_ns {
                break 'scan;
            }
            let day_start = local_day_start(record.start_ts_ns)?;
            if current_day_start != Some(day_start) {
                let day_end = next_local_day_start(day_start)?;
                let weekday = weekday_of_day_start(day_start)?;
                days.push(MiningDay {
                    day_start_ns: day_start,
                    day_end_ns: day_end,
                    weekday,
                    episodes: Vec::new(),
                });
                current_day_start = Some(day_start);
            }
            if let Some(day) = days.last_mut() {
                day.episodes.push(record);
            }
        }
        if !more {
            break;
        }
        let Some((last, _value)) = rows.last() else {
            break;
        };
        start = key_after(last);
    }
    Ok(days)
}

/// Existing `CF_ROUTINES` keys. A malformed key in derived state we own is
/// corruption to surface, never a row to skip or silently overwrite around.
fn existing_routine_keys(db: &Db, scanned_rows: &mut u64) -> Result<Vec<Vec<u8>>, ErrorData> {
    let mut keys = Vec::new();
    let mut start: Vec<u8> = Vec::new();
    loop {
        if usize::try_from(*scanned_rows).unwrap_or(usize::MAX) >= MAX_SCAN_ROWS_PER_CALL {
            return Err(internal(format!(
                "ROUTINE_SCAN_BUDGET_EXHAUSTED after {MAX_SCAN_ROWS_PER_CALL} CF_ROUTINES rows; \
                 the routine store should hold at most a few hundred rows — inspect CF_ROUTINES"
            )));
        }
        let (rows, more) = db
            .scan_cf_from(cf::CF_ROUTINES, &start, SCAN_CHUNK_ROWS)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        if rows.is_empty() {
            break;
        }
        for (key, _value) in &rows {
            *scanned_rows += 1;
            routine_codec::decode_routine_key(key).map_err(|error| {
                tracing::error!(
                    code = "ROUTINE_KEY_INVALID",
                    key_hex = %hex_encode(key),
                    %error,
                    "CF_ROUTINES holds a key its codec cannot decode"
                );
                mcp_error(
                    error_codes::STORAGE_READ_FAILED,
                    format!(
                        "ROUTINE_KEY_INVALID in CF_ROUTINES at {}: {error}; refusing to \
                         replace a store containing keys this codec cannot account for",
                        hex_encode(key)
                    ),
                )
            })?;
            keys.push(key.clone());
        }
        if !more {
            break;
        }
        let Some((last, _value)) = rows.last() else {
            break;
        };
        start = key_after(last);
    }
    Ok(keys)
}

/// Mines routines from `CF_EPISODES` and (unless `dry_run`) replaces
/// `CF_ROUTINES` atomically. Shared by the MCP tool and the periodic job.
#[allow(clippy::too_many_lines)]
pub fn mine_and_store_routines(
    db: &Arc<Db>,
    params: &RoutineMineParams,
) -> Result<RoutineMineResponse, ErrorData> {
    if let (Some(start), Some(end)) = (params.start_ts_ns, params.end_ts_ns)
        && start >= end
    {
        return Err(invalid(format!(
            "routine_mine start_ts_ns {start} must be < end_ts_ns {end}"
        )));
    }
    let config = build_config(params)?;

    let _mining = MINE_LOCK
        .lock()
        .map_err(|_poisoned| internal("routine mining lock poisoned"))?;

    if !params.dry_run && !db.pressure_permits_write(cf::CF_ROUTINES) {
        return Err(mcp_error(
            error_codes::STORAGE_WRITE_FAILED,
            format!(
                "routine_mine refused under disk pressure: cf_name={} pressure_level={:?}; \
                 nothing was deleted or written",
                cf::CF_ROUTINES,
                db.pressure_level()
            ),
        ));
    }

    let mut scanned_rows = 0_u64;
    let range_start = match params.start_ts_ns {
        Some(start) => start,
        None => match first_episode_ts(db, &mut scanned_rows)? {
            Some(ts_ns) => ts_ns,
            None => {
                // Empty episode store: an honest empty mine. A non-dry run
                // still clears stale routines (derived state must reflect
                // its source).
                let stale_keys = existing_routine_keys(db, &mut scanned_rows)?;
                let deleted = u64::try_from(stale_keys.len()).unwrap_or(u64::MAX);
                if !params.dry_run && !stale_keys.is_empty() {
                    db.mutate_batch_pressure_bypass(
                        cf::CF_ROUTINES,
                        stale_keys,
                        Vec::<(Vec<u8>, Vec<u8>)>::new(),
                    )
                    .map_err(|error| mcp_error(error.code(), error.to_string()))?;
                }
                return Ok(RoutineMineResponse {
                    range_start_ns: 0,
                    range_end_ns: 0,
                    scanned_episode_rows: scanned_rows,
                    considered_episodes: 0,
                    eligible_episodes: 0,
                    filtered_agent_episodes: 0,
                    filtered_short_episodes: 0,
                    filtered_no_app_episodes: 0,
                    active_days: 0,
                    candidates_evaluated: 0,
                    candidates_truncated: 0,
                    occurrences_skipped_over_cap: 0,
                    clusters_rejected_low_support: 0,
                    clusters_rejected_dispersed: 0,
                    clusters_rejected_low_confidence: 0,
                    candidates_rejected_as_subpattern: 0,
                    routines_dropped_over_cap: 0,
                    routines_written: 0,
                    routines_deleted: if params.dry_run { 0 } else { deleted },
                    dry_run: params.dry_run,
                    routines: Vec::new(),
                });
            }
        },
    };
    let range_end = params.end_ts_ns.unwrap_or_else(now_ts_ns);
    if range_start >= range_end {
        return Err(invalid(format!(
            "routine_mine effective range is empty: start {range_start} >= end {range_end}"
        )));
    }
    let range_start_snapped = local_day_start(range_start)?;
    let range_end_snapped = next_local_day_start(local_day_start(range_end.saturating_sub(1))?)?;

    let days = mining_days(
        db,
        range_start_snapped,
        range_end_snapped,
        &mut scanned_rows,
    )?;
    let mined_at = now_ts_ns();
    let mining = mine_routines(&days, mined_at, &config).map_err(|error| {
        internal(format!(
            "routine_mine engine failed for range [{range_start_snapped}, {range_end_snapped}): {error}"
        ))
    })?;

    let mut new_rows: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(mining.routines.len());
    for routine in &mining.routines {
        let key = routine_codec::routine_key(&routine.routine_id)
            .map_err(|error| internal(format!("engine produced an invalid routine id: {error}")))?;
        let value =
            encode_json(routine).map_err(|error| mcp_error(error.code(), error.to_string()))?;
        new_rows.push((key, value));
    }
    let written = u64::try_from(new_rows.len()).unwrap_or(u64::MAX);
    let mut deleted = 0_u64;
    if params.dry_run {
        tracing::info!(
            code = "ROUTINE_MINE_DRY_RUN",
            range_start_ns = range_start_snapped,
            range_end_ns = range_end_snapped,
            routines = written,
            "routine_mine dry run computed without mutating CF_ROUTINES"
        );
    } else {
        let stale_keys = existing_routine_keys(db, &mut scanned_rows)?;
        deleted = u64::try_from(stale_keys.len()).unwrap_or(u64::MAX);
        db.mutate_batch_pressure_bypass(cf::CF_ROUTINES, stale_keys, new_rows)
            .map_err(|error| {
                mcp_error(
                    error.code(),
                    format!(
                        "routine_mine failed to replace CF_ROUTINES: {error}; \
                         the previous routines are unchanged"
                    ),
                )
            })?;
        tracing::info!(
            code = "ROUTINE_MINE_REPLACED",
            range_start_ns = range_start_snapped,
            range_end_ns = range_end_snapped,
            routines_written = written,
            routines_deleted = deleted,
            active_days = mining.active_days,
            candidates = mining.candidates_evaluated,
            "routine_mine replaced the routine store"
        );
    }

    Ok(RoutineMineResponse {
        range_start_ns: range_start_snapped,
        range_end_ns: range_end_snapped,
        scanned_episode_rows: scanned_rows,
        considered_episodes: mining.considered_episodes,
        eligible_episodes: mining.eligible_episodes,
        filtered_agent_episodes: mining.filtered_agent_episodes,
        filtered_short_episodes: mining.filtered_short_episodes,
        filtered_no_app_episodes: mining.filtered_no_app_episodes,
        active_days: mining.active_days,
        candidates_evaluated: mining.candidates_evaluated,
        candidates_truncated: mining.candidates_truncated,
        occurrences_skipped_over_cap: mining.occurrences_skipped_over_cap,
        clusters_rejected_low_support: mining.clusters_rejected_low_support,
        clusters_rejected_dispersed: mining.clusters_rejected_dispersed,
        clusters_rejected_low_confidence: mining.clusters_rejected_low_confidence,
        candidates_rejected_as_subpattern: mining.candidates_rejected_as_subpattern,
        routines_dropped_over_cap: mining.routines_dropped_over_cap,
        routines_written: if params.dry_run { 0 } else { written },
        routines_deleted: deleted,
        dry_run: params.dry_run,
        routines: mining.routines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_validation_rejects_out_of_range_values() {
        let reject = |params: RoutineMineParams, fragment: &str| {
            let error = build_config(&params).expect_err(fragment);
            assert!(
                error.message.contains(fragment),
                "expected {fragment:?} in {:?}",
                error.message
            );
        };
        reject(
            RoutineMineParams {
                min_support_days: Some(0),
                ..RoutineMineParams::default()
            },
            "min_support_days",
        );
        reject(
            RoutineMineParams {
                min_support_days: Some(MIN_SUPPORT_DAYS_LIMIT + 1),
                ..RoutineMineParams::default()
            },
            "min_support_days",
        );
        reject(
            RoutineMineParams {
                max_pattern_len: Some(0),
                ..RoutineMineParams::default()
            },
            "max_pattern_len",
        );
        reject(
            RoutineMineParams {
                max_pattern_len: Some(MAX_PATTERN_LEN_LIMIT + 1),
                ..RoutineMineParams::default()
            },
            "max_pattern_len",
        );
        reject(
            RoutineMineParams {
                min_episode_duration_ms: Some(86_400_001),
                ..RoutineMineParams::default()
            },
            "min_episode_duration_ms",
        );
    }

    #[test]
    fn params_map_onto_engine_config() {
        let params = RoutineMineParams {
            min_support_days: Some(2),
            min_episode_duration_ms: Some(30_000),
            max_pattern_len: Some(4),
            include_agent_activity: true,
            ..RoutineMineParams::default()
        };
        let config = build_config(&params).expect("valid params");
        assert_eq!(config.min_support_days, 2);
        assert_eq!(config.min_episode_duration_ns, 30_000_000_000);
        assert_eq!(config.max_pattern_len, 4);
        assert!(config.include_agent_activity);
        let defaults = build_config(&RoutineMineParams::default()).expect("defaults");
        assert_eq!(defaults, RoutineMiningConfig::default());
    }

    #[test]
    fn weekday_helper_matches_chrono() {
        // 2026-06-08 was a Monday; local midnight of any instant that day
        // must map to weekday 0 in the local calendar.
        let monday_noon_utc = 1_780_920_000_000_000_000_u64; // 2026-06-08T12:00:00Z
        let day_start = local_day_start(monday_noon_utc).expect("day start");
        let weekday = weekday_of_day_start(day_start).expect("weekday");
        println!("weekday_helper day_start={day_start} weekday={weekday}");
        assert!(weekday <= 6);
        let ts = i64::try_from(day_start).expect("fits");
        assert_eq!(
            u32::from(weekday),
            Local.timestamp_nanos(ts).weekday().num_days_from_monday()
        );
    }
}
