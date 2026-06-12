use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Envelope schema version for [`RoutineRecord`] rows.
pub const ROUTINE_RECORD_VERSION: u32 = 1;

/// Identity granularity a routine was mined at (#848).
///
/// `App` patterns generalize across documents ("opens Excel every morning");
/// `AppDocument` patterns are document-specific ("opens report.xlsx every
/// morning"). Both passes run; closed-pattern suppression removes an `App`
/// routine that carries no information beyond an `AppDocument` one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutineGranularity {
    App,
    AppDocument,
}

/// Day-of-week classification of a routine's schedule signature (#848).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutineDowClass {
    /// Seen on at least six distinct weekdays.
    Daily,
    /// Seen on two or more distinct weekdays, Monday–Friday only.
    Weekdays,
    /// Seen on Saturday/Sunday only.
    Weekend,
    /// Explicit weekday list (0 = Monday … 6 = Sunday), sorted ascending.
    Days { days: Vec<u8> },
}

/// One ordered step of a routine's episode template.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutineStep {
    /// Lowercased process executable name.
    pub app: String,
    /// Lowercased document identity (URL host for browser episodes,
    /// normalized window title otherwise). `None` for `App`-granularity
    /// steps and episodes without a document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
}

/// One occurrence of the routine, kept as inspectable support evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutineEvidence {
    /// Local-midnight start of the day the occurrence happened on.
    pub day_start_ns: u64,
    /// Minute of that local day the first step started at.
    pub minute_of_day: u32,
    /// Stable episode ids (`ep1-…`) of the steps, in template order.
    pub episode_ids: Vec<String>,
}

/// One mined routine persisted in `CF_ROUTINES` (#848).
///
/// Routines are derived state: a pure, deterministic function of the
/// episode store and the mining config. Re-mining replaces all rows
/// atomically, so the store always reflects exactly one mining run.
/// `ts_ns` is the mining instant (the one engine input that varies between
/// runs); `routine_id` deliberately excludes it so re-mining the same
/// episodes reproduces the same ids.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutineRecord {
    pub record_version: u32,
    /// Mining instant (ns since epoch). `CF_ROUTINES` has no TTL; this is
    /// provenance, not a retention contract.
    pub ts_ns: u64,
    /// Stable deterministic id: `rt1-` + first 16 hex chars of SHA-256 over
    /// granularity, step keys, day-of-week class, and time-cluster ordinal.
    pub routine_id: String,
    pub granularity: RoutineGranularity,
    /// Ordered episode template (collapsed: consecutive identical
    /// identities merge into one step).
    pub steps: Vec<RoutineStep>,
    pub dow_class: RoutineDowClass,
    /// Circular mean start minute of the local day (0..1440).
    pub mean_minute_of_day: u32,
    /// Maximum circular deviation from the mean across occurrences.
    pub tolerance_minutes: u32,
    /// Human-readable schedule signature, e.g. `weekdays 08:45±20m`.
    pub schedule_label: String,
    /// Distinct local days the routine occurred on (the support count).
    pub support_days: u32,
    /// Total occurrences inside the time cluster (a day can hold several).
    pub occurrence_count: u32,
    /// Active days in the window matching `dow_class` — the denominator
    /// the confidence is computed against.
    pub opportunity_days: u32,
    /// Wilson 95% lower bound of `support_days / opportunity_days`;
    /// honest at low support by construction.
    pub confidence: f64,
    /// Day-snapped mining window this record was derived from.
    pub window_start_ns: u64,
    pub window_end_ns: u64,
    /// Days in the window with at least one eligible episode.
    pub active_days_in_window: u32,
    pub first_seen_day_start_ns: u64,
    pub last_seen_day_start_ns: u64,
    /// Most recent occurrences (capped), newest last.
    pub evidence: Vec<RoutineEvidence>,
}
