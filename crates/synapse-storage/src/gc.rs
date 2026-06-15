use std::{sync::Arc, time::Duration};

use rocksdb::{ColumnFamilyRef, DB, IteratorMode};
use synapse_core::{error_codes, retention::DEFAULTS};

use crate::{StorageError, StorageResult};

const GC_INTERVAL: Duration = Duration::from_mins(5);
const MIB: u64 = 1024 * 1024;
const ESTIMATE_LIVE_DATA_SIZE: &str = "rocksdb.estimate-live-data-size";
const ESTIMATE_NUM_KEYS: &str = "rocksdb.estimate-num-keys";
const CACHE_EVICTIONS_TOTAL: &str = "cache_evictions_total";
const SOFT_CAP_REASON: &str = "soft_cap";

/// One storage GC pass across all configured column families.
#[derive(Debug, Default)]
pub struct GcReport {
    pub cf_reports: Vec<GcCfReport>,
}

impl GcReport {
    /// Total rows evicted by this pass.
    #[must_use]
    pub fn total_evicted_rows(&self) -> u64 {
        self.cf_reports
            .iter()
            .map(|report| report.evicted_rows)
            .sum()
    }

    /// Finds the report for one column family.
    #[must_use]
    pub fn cf(&self, cf_name: &str) -> Option<&GcCfReport> {
        self.cf_reports
            .iter()
            .find(|report| report.cf_name == cf_name)
    }
}

/// Per-column-family GC outcome.
#[derive(Debug)]
pub struct GcCfReport {
    pub cf_name: String,
    pub before_value: u64,
    pub after_value: u64,
    pub before_estimated_num_keys: Option<u64>,
    pub after_estimated_num_keys: Option<u64>,
    pub evicted_rows: u64,
    pub hard_cap_reached: bool,
    pub hard_cap_code: Option<&'static str>,
}

/// Handle for the periodic storage GC task.
#[derive(Debug)]
pub struct GcTask {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for GcTask {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.handle.abort();
    }
}

#[derive(Clone, Debug)]
pub struct GcConfig {
    interval: Duration,
    budgets: Vec<GcBudget>,
}

impl GcConfig {
    pub fn from_retention_defaults() -> Self {
        Self {
            interval: GC_INTERVAL,
            budgets: DEFAULTS
                .iter()
                .map(|default| GcBudget {
                    cf_name: default.cf,
                    soft_cap: default.soft_cap_mb.saturating_mul(MIB),
                    hard_cap: default.hard_cap_mb.saturating_mul(MIB),
                    unit: CapUnit::Bytes,
                })
                .collect(),
        }
    }
}

impl GcConfig {
    pub(crate) fn for_row_caps(
        interval: Duration,
        cf_name: &'static str,
        soft_cap: u64,
        hard_cap: u64,
    ) -> Self {
        Self {
            interval,
            budgets: vec![GcBudget {
                cf_name,
                soft_cap,
                hard_cap,
                unit: CapUnit::Rows,
            }],
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct GcBudget {
    cf_name: &'static str,
    soft_cap: u64,
    hard_cap: u64,
    unit: CapUnit,
}

#[derive(Clone, Copy, Debug)]
enum CapUnit {
    Bytes,
    Rows,
}

pub fn spawn(db: Arc<DB>, config: GcConfig) -> StorageResult<GcTask> {
    let handle =
        tokio::runtime::Handle::try_current().map_err(|error| StorageError::WriteFailed {
            cf_name: "storage_gc".to_owned(),
            detail: error.to_string(),
        })?;
    let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let task = handle.spawn(async move {
        let mut interval = tokio::time::interval(config.interval);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(error) = run_once(&db, &config) {
                        tracing::warn!(error = %error, "storage GC tick failed");
                    }
                }
                _ = &mut shutdown_rx => break,
            }
        }
    });
    Ok(GcTask {
        shutdown: Some(shutdown),
        handle: task,
    })
}

pub fn run_once(db: &DB, config: &GcConfig) -> StorageResult<GcReport> {
    let mut cf_reports = Vec::with_capacity(config.budgets.len());
    for budget in &config.budgets {
        cf_reports.push(run_cf(db, *budget)?);
    }
    Ok(GcReport { cf_reports })
}

fn run_cf(db: &DB, budget: GcBudget) -> StorageResult<GcCfReport> {
    let cf = cf_handle(db, budget.cf_name)?;
    let keys = collect_keys(db, &cf, budget.cf_name)?;
    let before_estimated_num_keys = cf_property(db, &cf, budget.cf_name, ESTIMATE_NUM_KEYS)?;
    let before_value = measured_value(db, &cf, budget, keys.len() as u64)?;
    let hard_cap_reached = before_value >= budget.hard_cap;
    if hard_cap_reached {
        tracing::warn!(
            code = error_codes::STORAGE_CF_HARD_CAP_REACHED,
            cf = budget.cf_name,
            before_value,
            hard_cap = budget.hard_cap,
            "storage column family hard cap reached"
        );
    }

    let evicted_rows = if before_value > budget.soft_cap && !keys.is_empty() {
        evict_oldest(db, &cf, budget, &keys, before_value)?
    } else {
        0
    };
    let after_keys = collect_keys(db, &cf, budget.cf_name)?;
    let after_value = measured_value(db, &cf, budget, after_keys.len() as u64)?;
    let after_estimated_num_keys = cf_property(db, &cf, budget.cf_name, ESTIMATE_NUM_KEYS)?;

    if evicted_rows > 0 {
        synapse_telemetry::metrics::counter!(
            CACHE_EVICTIONS_TOTAL,
            "cf" => budget.cf_name,
            "reason" => SOFT_CAP_REASON
        )
        .increment(evicted_rows);
        tracing::info!(
            code = "STORAGE_CACHE_EVICTIONS_TOTAL_INCREMENTED",
            metric_name = CACHE_EVICTIONS_TOTAL,
            cf = budget.cf_name,
            reason = SOFT_CAP_REASON,
            delta = evicted_rows,
            before_value,
            after_value,
            "storage cache eviction counter incremented"
        );
    }

    Ok(GcCfReport {
        cf_name: budget.cf_name.to_owned(),
        before_value,
        after_value,
        before_estimated_num_keys,
        after_estimated_num_keys,
        evicted_rows,
        hard_cap_reached,
        hard_cap_code: hard_cap_reached.then_some(error_codes::STORAGE_CF_HARD_CAP_REACHED),
    })
}

fn evict_oldest(
    db: &DB,
    cf: &ColumnFamilyRef<'_>,
    budget: GcBudget,
    keys: &[Vec<u8>],
    before_value: u64,
) -> StorageResult<u64> {
    let remove_count = remove_count(budget, keys.len(), before_value);
    if remove_count == 0 {
        return Ok(0);
    }

    let start = keys
        .first()
        .ok_or_else(|| read_failed(budget.cf_name, "missing first key for GC range".to_owned()))?;
    let end = keys
        .get(remove_count)
        .cloned()
        .unwrap_or_else(|| keys.last().map_or_else(Vec::new, |last| key_after(last)));
    db.delete_range_cf(cf, start, &end)
        .map_err(|error| write_failed(budget.cf_name, error.to_string()))?;
    db.flush_cf(cf)
        .map_err(|error| write_failed(budget.cf_name, error.to_string()))?;
    db.compact_range_cf(cf, None::<&[u8]>, None::<&[u8]>);
    Ok(remove_count as u64)
}

fn remove_count(budget: GcBudget, key_count: usize, before_value: u64) -> usize {
    match budget.unit {
        // Explicit row cap (storage_gc_once / dashboard "cap at N"): evict exactly
        // the overage so the column family lands precisely at `soft_cap`. This is
        // an operator-driven, one-shot action — there is no minimum batch, so
        // capping a 140-row store at 136 removes 4 rows, not a forced quarter.
        CapUnit::Rows => {
            usize::try_from(before_value.saturating_sub(budget.soft_cap)).unwrap_or(usize::MAX)
        }
        // Byte/disk-pressure GC cannot map a byte overage onto an exact row count,
        // so it evicts a 25%-of-store batch per pass to amortize compaction across
        // the periodic ticks rather than thrash on every run.
        CapUnit::Bytes => key_count.div_ceil(4),
    }
    .min(key_count)
}

fn measured_value(
    db: &DB,
    cf: &ColumnFamilyRef<'_>,
    budget: GcBudget,
    exact_rows: u64,
) -> StorageResult<u64> {
    match budget.unit {
        CapUnit::Rows => Ok(exact_rows),
        CapUnit::Bytes => cf_property(db, cf, budget.cf_name, ESTIMATE_LIVE_DATA_SIZE)
            .map(Option::unwrap_or_default),
    }
}

fn collect_keys(db: &DB, cf: &ColumnFamilyRef<'_>, cf_name: &str) -> StorageResult<Vec<Vec<u8>>> {
    let mut keys = Vec::new();
    for item in db.iterator_cf(cf, IteratorMode::Start) {
        let (key, _value) = item.map_err(|error| read_failed(cf_name, error.to_string()))?;
        keys.push(key.to_vec());
    }
    Ok(keys)
}

fn cf_property(
    db: &DB,
    cf: &ColumnFamilyRef<'_>,
    cf_name: &str,
    property: &str,
) -> StorageResult<Option<u64>> {
    db.property_int_value_cf(cf, property)
        .map_err(|error| read_failed(cf_name, error.to_string()))
}

fn cf_handle<'db>(db: &'db DB, cf_name: &str) -> StorageResult<ColumnFamilyRef<'db>> {
    db.cf_handle(cf_name)
        .ok_or_else(|| read_failed(cf_name, "column family handle missing".to_owned()))
}

fn key_after(key: &[u8]) -> Vec<u8> {
    let mut end = key.to_vec();
    end.push(0);
    end
}

fn read_failed(cf_name: &str, detail: String) -> StorageError {
    StorageError::ReadFailed {
        cf_name: cf_name.to_owned(),
        detail,
    }
}

fn write_failed(cf_name: &str, detail: String) -> StorageError {
    StorageError::WriteFailed {
        cf_name: cf_name.to_owned(),
        detail,
    }
}
