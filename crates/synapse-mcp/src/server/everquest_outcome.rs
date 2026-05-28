use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use chrono::Utc;
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use synapse_core::error_codes;
use synapse_everquest::{EverQuestCompactOutcome, parse_log_file_name, parse_outcome_line};

use super::{
    Json, Parameters, SynapseService,
    everquest_log::{ActiveEverQuestLog, EVERQUEST_PROFILE_ID},
    tool, tool_router,
};
use crate::m1::mcp_error;

const TOOL: &str = "everquest_outcome_ingest";
const OUTCOME_ROW_PREFIX: &str = "everquest/outcome_event/v1";
const SCHEMA_VERSION: u32 = 1;
const DEFAULT_MAX_BYTES: usize = 64 * 1024;
const MAX_BYTES: usize = 512 * 1024;
const DEFAULT_MAX_EVENTS: usize = 64;
const MAX_EVENTS: usize = 256;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestOutcomeIngestParams {
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<u64>,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_max_events")]
    pub max_events: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    #[serde(default)]
    pub allow_explicit_log_path: bool,
    #[serde(default = "default_persist_unknown")]
    pub persist_unknown: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestOutcomeIngestResponse {
    pub ok: bool,
    pub row_prefix: String,
    pub source: EverQuestOutcomeIngestSource,
    pub rows_read: usize,
    pub rows_persisted: usize,
    pub duplicate_rows: usize,
    pub skipped_unknown_rows: usize,
    pub truncated_by_bytes: bool,
    pub truncated_by_events: bool,
    pub rows: Vec<EverQuestOutcomeRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestOutcomeIngestSource {
    pub path: String,
    pub start_offset: u64,
    pub next_offset: u64,
    pub file_len_bytes: u64,
    pub bytes_read: usize,
    pub truncated_by_bytes: bool,
    pub truncated_by_events: bool,
    pub explicit_log_path: bool,
    pub active_character: Option<String>,
    pub server: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestOutcomeRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub event_id: String,
    pub row_key: String,
    pub ingested_at: chrono::DateTime<Utc>,
    pub source: EverQuestOutcomeSourceRef,
    pub outcome: EverQuestCompactOutcome,
    pub duplicate_of_prior_row: bool,
    pub evidence_boundary: EverQuestOutcomeEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestOutcomeSourceRef {
    pub path: String,
    pub start_offset: u64,
    pub next_offset: u64,
    pub line_index_in_read: usize,
    pub content_sha256: String,
    pub content_len_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_timestamp: Option<String>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestOutcomeEvidenceBoundary {
    pub raw_chat_body_persisted: bool,
    pub compact_redacted: bool,
    pub source_hash_present: bool,
    pub manual_fsv_required_for_runtime: bool,
    pub note: String,
}

#[derive(Clone, Debug)]
struct ParsedOutcomeLine {
    line_index_in_read: usize,
    start_offset: u64,
    next_offset: u64,
    content: Vec<u8>,
}

#[tool_router(router = everquest_outcome_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Parse active or explicit EverQuest log bytes into compact redacted outcome rows and persist them in CF_KV with offset/hash readback"
    )]
    pub async fn everquest_outcome_ingest(
        &self,
        params: Parameters<EverQuestOutcomeIngestParams>,
    ) -> Result<Json<EverQuestOutcomeIngestResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=everquest_outcome_ingest"
        );
        let params = normalize_params(params.0)?;
        let active = self.resolve_active_everquest_log().map_err(|detail| {
            mcp_error(
                error_codes::ACTION_TARGET_INVALID,
                format!("{TOOL} could not resolve active EverQuest log: {detail}"),
            )
        })?;
        let explicit_log_path = params.log_path.is_some();
        let path = resolve_log_path(&params, &active)?;
        let (source, lines) = read_outcome_lines(&path, &active, &params, explicit_log_path)?;
        let response =
            self.persist_outcome_rows(&params.profile_id, source, lines, params.persist_unknown)?;
        Ok(Json(response))
    }
}

impl SynapseService {
    fn persist_outcome_rows(
        &self,
        profile_id: &str,
        source: EverQuestOutcomeIngestSource,
        lines: Vec<ParsedOutcomeLine>,
        persist_unknown: bool,
    ) -> Result<EverQuestOutcomeIngestResponse, ErrorData> {
        let mut rows = Vec::new();
        let mut skipped_unknown_rows = 0_usize;
        for line in lines {
            let line_text = String::from_utf8_lossy(&line.content);
            let outcome = parse_outcome_line(line_text.trim_end_matches('\r'));
            if !persist_unknown
                && matches!(
                    outcome.kind,
                    synapse_everquest::EverQuestOutcomeKind::Unknown
                        | synapse_everquest::EverQuestOutcomeKind::DiagnosticMissingTimestamp
                        | synapse_everquest::EverQuestOutcomeKind::DiagnosticMalformedTimestamp
                )
            {
                skipped_unknown_rows = skipped_unknown_rows.saturating_add(1);
                continue;
            }
            let content_sha256 = sha256_hex(&line.content);
            let event_id = format!("{:016x}-{}", line.start_offset, &content_sha256[..16]);
            let row_key = outcome_row_key(profile_id, &event_id);
            rows.push(EverQuestOutcomeRow {
                schema_version: SCHEMA_VERSION,
                row_kind: "everquest_outcome_event".to_owned(),
                profile_id: profile_id.to_owned(),
                event_id,
                row_key,
                ingested_at: Utc::now(),
                source: EverQuestOutcomeSourceRef {
                    path: source.path.clone(),
                    start_offset: line.start_offset,
                    next_offset: line.next_offset,
                    line_index_in_read: line.line_index_in_read,
                    content_sha256,
                    content_len_bytes: line.content.len(),
                    timestamp_text: outcome.timestamp_text.clone(),
                    log_timestamp: outcome
                        .timestamp
                        .map(|timestamp| timestamp.format("%Y-%m-%dT%H:%M:%S").to_string()),
                },
                outcome,
                duplicate_of_prior_row: false,
                evidence_boundary: evidence_boundary(),
            });
        }

        let (readback_rows, duplicate_rows) = self.put_and_readback_rows(rows)?;
        Ok(EverQuestOutcomeIngestResponse {
            ok: true,
            row_prefix: format!("{OUTCOME_ROW_PREFIX}/{profile_id}"),
            rows_read: readback_rows.len().saturating_add(skipped_unknown_rows),
            rows_persisted: readback_rows.len(),
            duplicate_rows,
            skipped_unknown_rows,
            truncated_by_bytes: source.truncated_by_bytes,
            truncated_by_events: source.truncated_by_events,
            source,
            rows: readback_rows,
        })
    }

    fn put_and_readback_rows(
        &self,
        mut rows: Vec<EverQuestOutcomeRow>,
    ) -> Result<(Vec<EverQuestOutcomeRow>, usize), ErrorData> {
        let mut duplicate_rows = 0_usize;
        let mut encoded_rows = Vec::with_capacity(rows.len());
        {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while writing EverQuest outcome rows",
                )
            })?;
            for row in &mut rows {
                let existing = runtime
                    .storage_kv_row(row.row_key.as_bytes())
                    .map_err(|error| mcp_error(error.code(), error.to_string()))?;
                if existing.is_some() {
                    duplicate_rows = duplicate_rows.saturating_add(1);
                    row.duplicate_of_prior_row = true;
                }
                encoded_rows.push((
                    row.row_key.as_bytes().to_vec(),
                    serde_json::to_vec(row).map_err(|error| {
                        mcp_error(
                            error_codes::TOOL_INTERNAL_ERROR,
                            format!("encode EverQuest outcome row: {error}"),
                        )
                    })?,
                ));
            }
            if !encoded_rows.is_empty() {
                runtime.storage_put_kv_rows(encoded_rows).map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_WRITE_FAILED,
                        format!("write EverQuest outcome rows: {error}"),
                    )
                })?;
            }
            let mut readback_rows = Vec::with_capacity(rows.len());
            for row in rows {
                let stored = runtime
                    .storage_kv_row(row.row_key.as_bytes())
                    .map_err(|error| {
                        mcp_error(
                            error_codes::STORAGE_READ_FAILED,
                            format!("read EverQuest outcome row after write: {error}"),
                        )
                    })?
                    .ok_or_else(|| {
                        mcp_error(
                            error_codes::STORAGE_READ_FAILED,
                            format!("EverQuest outcome row missing after write: {}", row.row_key),
                        )
                    })?;
                readback_rows.push(decode_json_row::<EverQuestOutcomeRow>(
                    &stored,
                    "EverQuest outcome row",
                )?);
            }
            drop(runtime);
            Ok((readback_rows, duplicate_rows))
        }
    }
}

fn normalize_params(
    params: EverQuestOutcomeIngestParams,
) -> Result<EverQuestOutcomeIngestParams, ErrorData> {
    let profile_id = validate_profile_id(&params.profile_id)?;
    if params.max_bytes == 0 || params.max_bytes > MAX_BYTES {
        return Err(params_error(format!(
            "max_bytes must be between 1 and {MAX_BYTES}"
        )));
    }
    if params.max_events == 0 || params.max_events > MAX_EVENTS {
        return Err(params_error(format!(
            "max_events must be between 1 and {MAX_EVENTS}"
        )));
    }
    let log_path = params
        .log_path
        .map(|path| validate_path_text("log_path", &path))
        .transpose()?;
    Ok(EverQuestOutcomeIngestParams {
        profile_id,
        start_offset: params.start_offset,
        max_bytes: params.max_bytes,
        max_events: params.max_events,
        log_path,
        allow_explicit_log_path: params.allow_explicit_log_path,
        persist_unknown: params.persist_unknown,
    })
}

fn resolve_log_path(
    params: &EverQuestOutcomeIngestParams,
    active: &ActiveEverQuestLog,
) -> Result<PathBuf, ErrorData> {
    let Some(path_text) = params.log_path.as_deref() else {
        return Ok(active.log.path.clone());
    };
    if !params.allow_explicit_log_path {
        return Err(params_error(
            "log_path requires allow_explicit_log_path=true so explicit-file FSV is intentional",
        ));
    }
    let path = PathBuf::from(path_text);
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return Err(params_error("log_path must have a file name"));
    };
    if parse_log_file_name(file_name).is_none() {
        return Err(params_error(
            "log_path file name must match eqlog_<character>_<server>.txt",
        ));
    }
    Ok(path)
}

fn read_outcome_lines(
    path: &Path,
    active: &ActiveEverQuestLog,
    params: &EverQuestOutcomeIngestParams,
    explicit_log_path: bool,
) -> Result<(EverQuestOutcomeIngestSource, Vec<ParsedOutcomeLine>), ErrorData> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!("read EverQuest outcome log metadata: {error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!(
                "EverQuest outcome log path is not a file: {}",
                path.display()
            ),
        ));
    }
    let file_len_bytes = metadata.len();
    let start_offset = params.start_offset.unwrap_or_else(|| {
        file_len_bytes.saturating_sub(u64::try_from(params.max_bytes).unwrap_or(u64::MAX))
    });
    if start_offset > file_len_bytes {
        return Err(params_error(format!(
            "start_offset {start_offset} is beyond file length {file_len_bytes}"
        )));
    }
    let remaining = file_len_bytes.saturating_sub(start_offset);
    let read_len = usize::try_from(remaining)
        .unwrap_or(usize::MAX)
        .min(params.max_bytes);
    let mut file = File::open(path).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!("open EverQuest outcome log: {error}"),
        )
    })?;
    file.seek(SeekFrom::Start(start_offset)).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!("seek EverQuest outcome log: {error}"),
        )
    })?;
    let mut bytes = vec![0_u8; read_len];
    let bytes_read = file.read(&mut bytes).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!("read EverQuest outcome log: {error}"),
        )
    })?;
    bytes.truncate(bytes_read);
    let next_offset = start_offset.saturating_add(u64::try_from(bytes_read).unwrap_or(u64::MAX));
    let truncated_by_bytes = bytes_read == params.max_bytes
        && remaining > u64::try_from(params.max_bytes).unwrap_or(u64::MAX);
    let (lines, truncated_by_events) = parsed_lines(&bytes, start_offset, params.max_events);
    Ok((
        EverQuestOutcomeIngestSource {
            path: path.display().to_string(),
            start_offset,
            next_offset,
            file_len_bytes,
            bytes_read,
            truncated_by_bytes,
            truncated_by_events,
            explicit_log_path,
            active_character: active.active_character.clone(),
            server: active.log.identity.server.clone(),
        },
        lines,
    ))
}

fn parsed_lines(
    bytes: &[u8],
    start_offset: u64,
    max_events: usize,
) -> (Vec<ParsedOutcomeLine>, bool) {
    let mut rows = Vec::new();
    let mut line_start = 0_usize;
    let mut line_index = 0_usize;
    let mut truncated_by_events = false;
    while line_start < bytes.len() {
        if rows.len() == max_events {
            truncated_by_events = true;
            break;
        }
        let relative_end = bytes[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |position| line_start + position);
        let next_relative = if relative_end < bytes.len() {
            relative_end.saturating_add(1)
        } else {
            relative_end
        };
        let mut content_end = relative_end;
        if content_end > line_start && bytes[content_end - 1] == b'\r' {
            content_end -= 1;
        }
        let content = bytes[line_start..content_end].to_vec();
        if !content.is_empty() {
            rows.push(ParsedOutcomeLine {
                line_index_in_read: line_index,
                start_offset: start_offset
                    .saturating_add(u64::try_from(line_start).unwrap_or(u64::MAX)),
                next_offset: start_offset
                    .saturating_add(u64::try_from(next_relative).unwrap_or(u64::MAX)),
                content,
            });
        }
        line_start = next_relative;
        line_index = line_index.saturating_add(1);
    }
    (rows, truncated_by_events)
}

fn outcome_row_key(profile_id: &str, event_id: &str) -> String {
    format!("{OUTCOME_ROW_PREFIX}/{profile_id}/{event_id}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

fn decode_json_row<T>(bytes: &[u8], label: &str) -> Result<T, ErrorData>
where
    T: DeserializeOwned,
{
    serde_json::from_slice::<T>(bytes).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("decode {label}: {error}"),
        )
    })
}

fn evidence_boundary() -> EverQuestOutcomeEvidenceBoundary {
    EverQuestOutcomeEvidenceBoundary {
        raw_chat_body_persisted: false,
        compact_redacted: true,
        source_hash_present: true,
        manual_fsv_required_for_runtime: true,
        note: "Compact outcome rows preserve source offsets and hashes but do not persist raw chat bodies; manual FSV still reads physical log bytes and CF_KV rows."
            .to_owned(),
    }
}

fn validate_profile_id(value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value != EVERQUEST_PROFILE_ID {
        return Err(params_error(format!(
            "profile_id must be {EVERQUEST_PROFILE_ID:?}; got {value:?}"
        )));
    }
    Ok(value.to_owned())
}

fn validate_path_text(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(format!("{field} must not be empty")));
    }
    if value.len() > MAX_PATH_BYTES {
        return Err(params_error(format!(
            "{field} must be <= {MAX_PATH_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(params_error(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(value.to_owned())
}

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message)
}

fn default_profile_id() -> String {
    EVERQUEST_PROFILE_ID.to_owned()
}

const fn default_max_bytes() -> usize {
    DEFAULT_MAX_BYTES
}

const fn default_max_events() -> usize {
    DEFAULT_MAX_EVENTS
}

const fn default_persist_unknown() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_lines_preserve_offsets_and_skip_empty_lines() {
        let (lines, truncated) = parsed_lines(b"a\r\n\nbb\n", 10, 8);
        assert!(!truncated);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_offset, 10);
        assert_eq!(lines[0].next_offset, 13);
        assert_eq!(lines[0].content, b"a");
        assert_eq!(lines[1].start_offset, 14);
        assert_eq!(lines[1].next_offset, 17);
        assert_eq!(lines[1].content, b"bb");
    }

    #[test]
    fn row_key_is_stable_for_duplicate_cursor_range() {
        let hash = sha256_hex(b"[Thu May 28 11:00:00 2026] You gain experience!!");
        let event_id = format!("{:016x}-{}", 42_u64, &hash[..16]);
        assert_eq!(
            outcome_row_key(EVERQUEST_PROFILE_ID, &event_id),
            format!("everquest/outcome_event/v1/everquest.live/{event_id}")
        );
    }
}
