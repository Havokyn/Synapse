use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use chrono::Utc;
use serde_json::{Value, json};
use synapse_core::{EventSource, EventSummary};
use synapse_everquest::{
    EverQuestLogEvent, EverQuestLogFile, EverQuestLogKind, EverQuestLogTailBatch,
    discover_log_files, tail_log,
};
use synapse_perception::ObservationInput;

use crate::m1::EverQuestLogCursorState;

use super::SynapseService;

const EVERQUEST_PROFILE_ID: &str = "everquest.live";
const EVERQUEST_INSTALL_PATH_KEY: &str = "runtime.everquest.install_path";
const EVERQUEST_SERVER_KEY: &str = "runtime.everquest.server";
const EQCLIENT_FILE: &str = "eqclient.ini";
const EQCLIENT_LAST_CHAR_SEL: &str = "LastCharSel=";
const EQCLIENT_LOG: &str = "Log=";
const MAX_LOG_TAIL_BYTES: usize = 8 * 1024;
const MAX_LOG_TAIL_EVENTS: usize = 8;

impl SynapseService {
    pub(super) fn populate_everquest_log_events(&self, input: &mut ObservationInput) {
        if input.foreground.profile_id.as_deref() != Some(EVERQUEST_PROFILE_ID) {
            return;
        }

        let started = Instant::now();
        match self.read_everquest_log_events() {
            Ok(mut events) => input.recent_events.append(&mut events),
            Err(error) => input.recent_events.push(self.everquest_log_error_event(
                "everquest.log_error",
                "EVERQUEST_LOG_READ_FAILED",
                &error,
            )),
        }
        input.sensor_latency_ms.insert(
            "everquest_log".to_owned(),
            started.elapsed().as_secs_f32() * 1000.0,
        );
    }

    fn read_everquest_log_events(&self) -> Result<Vec<EventSummary>, String> {
        let active = self.resolve_active_everquest_log()?;
        let file_len = fs::metadata(&active.log.path)
            .map_err(|error| format!("read active EverQuest log metadata: {error}"))?
            .len();

        let cursor = {
            let state = self
                .m1_state()
                .map_err(|error| format!("read M1 cursor state: {}", error.message))?;
            state.everquest_log_cursor.clone()
        };

        let Some(cursor) = cursor.filter(|cursor| cursor.path == active.log.path) else {
            let mut state = self.m1_state().map_err(|error| {
                format!("write initialized EverQuest log cursor: {}", error.message)
            })?;
            state.everquest_log_cursor = Some(EverQuestLogCursorState {
                path: active.log.path.clone(),
                offset: file_len,
            });
            let seq = next_everquest_event_seq(&mut state);
            drop(state);
            return Ok(vec![cursor_event(
                seq,
                "everquest.log_cursor_initialized",
                &active,
                file_len,
                file_len,
                file_len,
                0,
                false,
                false,
                0,
                None,
            )]);
        };

        let mut rotated_from = None;
        let start_offset = if cursor.offset > file_len {
            rotated_from = Some(cursor.offset);
            0
        } else {
            cursor.offset
        };
        let batch = tail_log(
            &active.log.path,
            start_offset,
            MAX_LOG_TAIL_BYTES,
            MAX_LOG_TAIL_EVENTS,
        )
        .map_err(|error| format!("tail active EverQuest log: {error}"))?;

        let mut state = self
            .m1_state()
            .map_err(|error| format!("write EverQuest log cursor: {}", error.message))?;
        state.everquest_log_cursor = Some(EverQuestLogCursorState {
            path: active.log.path.clone(),
            offset: batch.next_offset,
        });

        let mut summaries = Vec::with_capacity(batch.events.len().saturating_add(2));
        if let Some(previous_offset) = rotated_from {
            summaries.push(cursor_event(
                next_everquest_event_seq(&mut state),
                "everquest.log_rotated",
                &active,
                previous_offset,
                batch.next_offset,
                batch.file_len_bytes,
                batch.bytes_read,
                batch.truncated_by_bytes,
                batch.truncated_by_events,
                batch.events.len(),
                Some("cursor was beyond current file length; treated as rotated/truncated"),
            ));
        }
        summaries.push(batch_cursor_event(
            next_everquest_event_seq(&mut state),
            "everquest.log_cursor",
            &active,
            &batch,
        ));
        for event in &batch.events {
            summaries.push(log_event_summary(
                next_everquest_event_seq(&mut state),
                &active,
                &batch,
                event,
            ));
        }
        drop(state);
        Ok(summaries)
    }

    fn resolve_active_everquest_log(&self) -> Result<ActiveEverQuestLog, String> {
        let runtime = self
            .profile_runtime()
            .map_err(|error| format!("load profile runtime: {}", error.message))?;
        let profile = runtime
            .profile(EVERQUEST_PROFILE_ID)
            .map_err(|error| format!("load EverQuest profile: {error}"))?
            .ok_or_else(|| "EverQuest profile is not loaded".to_owned())?;
        let install_root = profile
            .metadata
            .get(EVERQUEST_INSTALL_PATH_KEY)
            .map(PathBuf::from)
            .ok_or_else(|| "EverQuest profile has no install path metadata".to_owned())?;
        let expected_server = profile
            .metadata
            .get(EVERQUEST_SERVER_KEY)
            .map(|value| value.to_ascii_lowercase());
        let eqclient = read_eqclient_state(&install_root)?;
        if eqclient.log_enabled == Some(false) {
            return Err(format!(
                "EverQuest logging is disabled in {} ({EQCLIENT_LOG}0)",
                install_root.join(EQCLIENT_FILE).display()
            ));
        }
        let active_character = eqclient.last_char_sel;
        let logs = discover_log_files(&install_root)
            .map_err(|error| format!("discover EverQuest logs: {error}"))?;
        let log = choose_active_log(
            logs,
            expected_server.as_deref(),
            active_character.as_deref(),
        )
        .ok_or_else(|| {
            format!("no active EverQuest log matched character={active_character:?} server={expected_server:?}")
        })?;
        Ok(ActiveEverQuestLog {
            log,
            install_root,
            active_character,
            expected_server,
            log_enabled: eqclient.log_enabled,
        })
    }

    fn everquest_log_error_event(&self, kind: &str, code: &str, detail: &str) -> EventSummary {
        let seq = self
            .m1_state()
            .map(|mut state| next_everquest_event_seq(&mut state))
            .unwrap_or_default();
        EventSummary {
            seq,
            at: Utc::now(),
            source: EventSource::Filesystem,
            kind: kind.to_owned(),
            data_excerpt: json!({
                "code": code,
                "detail": detail,
            }),
        }
    }
}

#[derive(Clone, Debug)]
struct ActiveEverQuestLog {
    log: EverQuestLogFile,
    install_root: PathBuf,
    active_character: Option<String>,
    expected_server: Option<String>,
    log_enabled: Option<bool>,
}

fn choose_active_log(
    logs: Vec<EverQuestLogFile>,
    expected_server: Option<&str>,
    active_character: Option<&str>,
) -> Option<EverQuestLogFile> {
    let mut candidates = logs
        .into_iter()
        .filter(|log| {
            expected_server.is_none_or(|server| log.identity.server.eq_ignore_ascii_case(server))
        })
        .filter(|log| {
            active_character
                .is_none_or(|character| log.identity.character.eq_ignore_ascii_case(character))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        modified_time(&left.path)
            .cmp(&modified_time(&right.path))
            .then_with(|| left.path.cmp(&right.path))
    });
    candidates.pop()
}

fn modified_time(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct EqClientState {
    last_char_sel: Option<String>,
    log_enabled: Option<bool>,
}

fn read_eqclient_state(install_root: &Path) -> Result<EqClientState, String> {
    let path = install_root.join(EQCLIENT_FILE);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("read EverQuest client config {}: {error}", path.display()))?;
    let last_char_sel = text.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix(EQCLIENT_LAST_CHAR_SEL)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    });
    let log_enabled = text
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix(EQCLIENT_LOG)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .map(|value| parse_eqclient_bool(&value))
        .transpose()
        .map_err(|value| {
            format!(
                "EverQuest client config {} has unsupported {EQCLIENT_LOG}{value:?}",
                path.display()
            )
        })?;
    Ok(EqClientState {
        last_char_sel,
        log_enabled,
    })
}

fn parse_eqclient_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        other => Err(other.to_owned()),
    }
}

const fn next_everquest_event_seq(state: &mut crate::m1::M1State) -> u64 {
    let seq = state.everquest_event_seq;
    state.everquest_event_seq = state.everquest_event_seq.saturating_add(1);
    seq
}

#[allow(clippy::too_many_arguments)]
fn cursor_event(
    seq: u64,
    kind: &str,
    active: &ActiveEverQuestLog,
    start_offset: u64,
    next_offset: u64,
    file_len_bytes: u64,
    bytes_read: usize,
    truncated_by_bytes: bool,
    truncated_by_events: bool,
    event_count: usize,
    note: Option<&str>,
) -> EventSummary {
    EventSummary {
        seq,
        at: Utc::now(),
        source: EventSource::Filesystem,
        kind: kind.to_owned(),
        data_excerpt: cursor_data(
            active,
            start_offset,
            next_offset,
            file_len_bytes,
            bytes_read,
            truncated_by_bytes,
            truncated_by_events,
            event_count,
            note,
        ),
    }
}

fn batch_cursor_event(
    seq: u64,
    kind: &str,
    active: &ActiveEverQuestLog,
    batch: &EverQuestLogTailBatch,
) -> EventSummary {
    cursor_event(
        seq,
        kind,
        active,
        batch.start_offset,
        batch.next_offset,
        batch.file_len_bytes,
        batch.bytes_read,
        batch.truncated_by_bytes,
        batch.truncated_by_events,
        batch.events.len(),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn cursor_data(
    active: &ActiveEverQuestLog,
    start_offset: u64,
    next_offset: u64,
    file_len_bytes: u64,
    bytes_read: usize,
    truncated_by_bytes: bool,
    truncated_by_events: bool,
    event_count: usize,
    note: Option<&str>,
) -> Value {
    let mut value = json!({
        "path": active.log.path,
        "install_root": active.install_root,
        "character": active.log.identity.character,
        "server": active.log.identity.server,
        "active_character": active.active_character,
        "expected_server": active.expected_server,
        "log_enabled": active.log_enabled,
        "cursor": {
            "start_offset": start_offset,
            "next_offset": next_offset,
            "file_len_bytes": file_len_bytes,
            "bytes_read": bytes_read,
            "truncated_by_bytes": truncated_by_bytes,
            "truncated_by_events": truncated_by_events,
            "event_count": event_count,
        }
    });
    if let Some(note) = note {
        value["note"] = Value::String(note.to_owned());
    }
    value
}

fn log_event_summary(
    seq: u64,
    active: &ActiveEverQuestLog,
    batch: &EverQuestLogTailBatch,
    event: &EverQuestLogEvent,
) -> EventSummary {
    EventSummary {
        seq,
        at: Utc::now(),
        source: EventSource::Filesystem,
        kind: format!("everquest.log.{}", kind_name(&event.kind)),
        data_excerpt: json!({
            "path": active.log.path,
            "character": active.log.identity.character,
            "server": active.log.identity.server,
            "cursor": {
                "start_offset": batch.start_offset,
                "next_offset": batch.next_offset,
                "file_len_bytes": batch.file_len_bytes,
            },
            "log_timestamp": event.timestamp.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "event": {
                "kind": event.kind,
                "actor": event.actor,
                "target": event.target,
                "channel": event.channel,
                "level": event.level,
                "summary": safe_event_summary(event),
                "redacted": event_body_redacted(&event.kind),
            }
        }),
    }
}

const fn kind_name(kind: &EverQuestLogKind) -> &'static str {
    match kind {
        EverQuestLogKind::LoggingEnabled => "logging_enabled",
        EverQuestLogKind::TargetNpc => "target_npc",
        EverQuestLogKind::TargetPlayer => "target_player",
        EverQuestLogKind::TargetCleared => "target_cleared",
        EverQuestLogKind::Consider => "consider",
        EverQuestLogKind::CastBegins => "cast_begins",
        EverQuestLogKind::CastResult => "cast_result",
        EverQuestLogKind::Say => "say",
        EverQuestLogKind::Tell => "tell",
        EverQuestLogKind::System => "system",
        EverQuestLogKind::Other => "other",
    }
}

fn safe_event_summary(event: &EverQuestLogEvent) -> String {
    match event.kind {
        EverQuestLogKind::System => "system event".to_owned(),
        EverQuestLogKind::Other => "other event".to_owned(),
        _ => event.summary.clone(),
    }
}

const fn event_body_redacted(kind: &EverQuestLogKind) -> bool {
    matches!(
        kind,
        EverQuestLogKind::Say
            | EverQuestLogKind::Tell
            | EverQuestLogKind::System
            | EverQuestLogKind::Other
    )
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    #[test]
    fn runtime_log_summary_suppresses_chat_body() {
        let event = EverQuestLogEvent {
            timestamp: NaiveDate::from_ymd_opt(2026, 5, 28)
                .and_then(|date| date.and_hms_opt(8, 0, 0))
                .unwrap_or_else(|| panic!("valid timestamp")),
            kind: EverQuestLogKind::Tell,
            actor: Some("Mikaylah".to_owned()),
            target: None,
            channel: Some("general3:2".to_owned()),
            level: None,
            summary: "Mikaylah tells general3:2".to_owned(),
        };
        assert_eq!(safe_event_summary(&event), "Mikaylah tells general3:2");
        assert!(event_body_redacted(&event.kind));
    }

    #[test]
    fn eqclient_state_reads_character_and_disabled_log() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(
            dir.path().join(EQCLIENT_FILE),
            "LastCharSel=Thenumberone\r\nLog=0\r\n",
        )?;
        let state = read_eqclient_state(dir.path()).map_err(anyhow::Error::msg)?;
        assert_eq!(state.last_char_sel.as_deref(), Some("Thenumberone"));
        assert_eq!(state.log_enabled, Some(false));
        Ok(())
    }
}
