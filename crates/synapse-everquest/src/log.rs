use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use chrono::NaiveDateTime;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const LOG_DIR_NAME: &str = "Logs";
const MAX_SUMMARY_CHARS: usize = 160;

#[derive(Debug, Error)]
pub enum EverQuestLogError {
    #[error("EverQuest log path {path} is invalid: {reason}")]
    InvalidPath { path: PathBuf, reason: String },
    #[error("I/O error while reading EverQuest log path {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("EverQuest log line timestamp {timestamp:?} could not be parsed")]
    Timestamp { timestamp: String },
    #[error("EverQuest location log line could not be parsed: {message}")]
    Location { message: String },
    #[error("EverQuest zone-entry log line could not be parsed: {message}")]
    Zone { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EverQuestLogIdentity {
    pub character: String,
    pub server: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EverQuestLogFile {
    pub path: PathBuf,
    pub identity: EverQuestLogIdentity,
    pub len_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestLogKind {
    LoggingEnabled,
    Location,
    ZoneEntered,
    TargetNpc,
    TargetPlayer,
    TargetCleared,
    Consider,
    CastBegins,
    CastResult,
    Say,
    Tell,
    System,
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EverQuestLocation {
    pub display_y: f64,
    pub display_x: f64,
    pub display_z: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EverQuestLogEvent {
    pub timestamp: NaiveDateTime,
    pub kind: EverQuestLogKind,
    pub actor: Option<String>,
    pub target: Option<String>,
    pub channel: Option<String>,
    pub level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EverQuestLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EverQuestLogTailBatch {
    pub path: PathBuf,
    pub start_offset: u64,
    pub next_offset: u64,
    pub file_len_bytes: u64,
    pub bytes_read: usize,
    pub truncated_by_bytes: bool,
    pub truncated_by_events: bool,
    pub events: Vec<EverQuestLogEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestOutcomeKind {
    CombatDamageDealt,
    CombatDamageTaken,
    SpellBegins,
    SpellHit,
    SpellFizzle,
    SpellResist,
    XpGain,
    LevelUp,
    Death,
    Respawn,
    Loot,
    RestSit,
    TargetNpc,
    TargetPlayer,
    TargetCleared,
    Consider,
    ZoneEntered,
    Location,
    HazardSignal,
    ChatRedacted,
    AmbiguousCombat,
    Unknown,
    DiagnosticMalformedTimestamp,
    DiagnosticMissingTimestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EverQuestCompactOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<NaiveDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_text: Option<String>,
    pub kind: EverQuestOutcomeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spell: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EverQuestLocation>,
    pub summary: String,
    pub redacted: bool,
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_code: Option<String>,
}

#[must_use]
pub fn parse_log_file_name(name: &str) -> Option<EverQuestLogIdentity> {
    let rest = name.strip_prefix("eqlog_")?;
    let rest = rest.strip_suffix(".txt")?;
    let (character, server) = rest.rsplit_once('_')?;
    if character.trim().is_empty() || server.trim().is_empty() {
        return None;
    }
    Some(EverQuestLogIdentity {
        character: character.to_owned(),
        server: server.to_owned(),
    })
}

/// Discovers `EverQuest` character logs below an install root.
///
/// # Errors
///
/// Returns [`EverQuestLogError::InvalidPath`] when the root has no `Logs`
/// directory and [`EverQuestLogError::Io`] when the directory or file metadata
/// cannot be read.
pub fn discover_log_files(root: &Path) -> Result<Vec<EverQuestLogFile>, EverQuestLogError> {
    let log_dir = root.join(LOG_DIR_NAME);
    if !log_dir.is_dir() {
        return Err(EverQuestLogError::InvalidPath {
            path: log_dir,
            reason: "Logs directory is absent".to_owned(),
        });
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(&log_dir).map_err(|source| EverQuestLogError::Io {
        path: log_dir.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| EverQuestLogError::Io {
            path: log_dir.clone(),
            source,
        })?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(identity) = parse_log_file_name(name) else {
            continue;
        };
        let metadata = entry.metadata().map_err(|source| EverQuestLogError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.is_file() {
            files.push(EverQuestLogFile {
                path,
                identity,
                len_bytes: metadata.len(),
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

/// Parses one `EverQuest` log line into a compact event.
///
/// # Errors
///
/// Returns [`EverQuestLogError::Timestamp`] when a timestamped log line has a
/// timestamp that does not match `EverQuest`'s local log format.
pub fn parse_log_line(line: &str) -> Result<Option<EverQuestLogEvent>, EverQuestLogError> {
    let Some(captures) = line_regex().captures(line) else {
        return Ok(None);
    };
    let timestamp_text = captures
        .name("timestamp")
        .map(|value| value.as_str())
        .unwrap_or_default();
    let timestamp =
        NaiveDateTime::parse_from_str(timestamp_text, "%a %b %d %H:%M:%S %Y").map_err(|_| {
            EverQuestLogError::Timestamp {
                timestamp: timestamp_text.to_owned(),
            }
        })?;
    let message = captures
        .name("message")
        .map(|value| value.as_str())
        .unwrap_or_default()
        .trim();
    if message.starts_with(location_prefix()) {
        return parse_location_message(timestamp, message).map(Some);
    }
    if message.starts_with(zone_entered_prefix()) {
        return parse_zone_entered_message(timestamp, message).map(Some);
    }
    Ok(Some(classify_event(timestamp, message)))
}

#[must_use]
pub fn parse_outcome_line(line: &str) -> EverQuestCompactOutcome {
    let Some(captures) = line_regex().captures(line) else {
        return diagnostic_outcome(
            EverQuestOutcomeKind::DiagnosticMissingTimestamp,
            None,
            "missing_timestamp",
            "line did not match EverQuest timestamp format",
        );
    };
    let timestamp_text = captures
        .name("timestamp")
        .map(|value| value.as_str())
        .unwrap_or_default();
    let message = captures
        .name("message")
        .map(|value| value.as_str())
        .unwrap_or_default()
        .trim();
    let Ok(timestamp) = NaiveDateTime::parse_from_str(timestamp_text, "%a %b %d %H:%M:%S %Y")
    else {
        return diagnostic_outcome(
            EverQuestOutcomeKind::DiagnosticMalformedTimestamp,
            Some(timestamp_text),
            "malformed_timestamp",
            "timestamp did not parse",
        );
    };
    classify_outcome(timestamp, timestamp_text, message)
}

/// Reads new `EverQuest` log bytes from a cursor and returns compact events.
///
/// # Errors
///
/// Returns [`EverQuestLogError::InvalidPath`] when `path` is not a file,
/// [`EverQuestLogError::Io`] for filesystem read/seek failures, and
/// [`EverQuestLogError::Timestamp`] for malformed timestamped log lines.
pub fn tail_log(
    path: &Path,
    cursor: u64,
    max_bytes: usize,
    max_events: usize,
) -> Result<EverQuestLogTailBatch, EverQuestLogError> {
    let metadata = fs::metadata(path).map_err(|source| EverQuestLogError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(EverQuestLogError::InvalidPath {
            path: path.to_path_buf(),
            reason: "path is not a file".to_owned(),
        });
    }

    let file_len_bytes = metadata.len();
    let start_offset = cursor.min(file_len_bytes);
    let remaining = file_len_bytes.saturating_sub(start_offset);
    let read_len = usize::try_from(remaining)
        .unwrap_or(usize::MAX)
        .min(max_bytes);

    let mut file = File::open(path).map_err(|source| EverQuestLogError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.seek(SeekFrom::Start(start_offset))
        .map_err(|source| EverQuestLogError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut bytes = vec![0_u8; read_len];
    let bytes_read = file
        .read(&mut bytes)
        .map_err(|source| EverQuestLogError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    bytes.truncate(bytes_read);

    let text = String::from_utf8_lossy(&bytes);
    let mut events = Vec::new();
    let mut truncated_by_events = false;
    for line in text.lines() {
        if let Some(event) = parse_log_line(line)? {
            if events.len() == max_events {
                truncated_by_events = true;
                break;
            }
            events.push(event);
        }
    }

    Ok(EverQuestLogTailBatch {
        path: path.to_path_buf(),
        start_offset,
        next_offset: start_offset.saturating_add(u64::try_from(bytes_read).unwrap_or(u64::MAX)),
        file_len_bytes,
        bytes_read,
        truncated_by_bytes: bytes_read == max_bytes
            && remaining > u64::try_from(max_bytes).unwrap_or(u64::MAX),
        truncated_by_events,
        events,
    })
}

fn classify_event(timestamp: NaiveDateTime, message: &str) -> EverQuestLogEvent {
    if let Some(event) = classify_logging_or_target(timestamp, message) {
        return event;
    }
    if let Some(event) = classify_consider(timestamp, message) {
        return event;
    }
    if let Some(event) = classify_casting(timestamp, message) {
        return event;
    }
    if let Some(event) = classify_speech(timestamp, message) {
        return event;
    }
    let kind = if message.starts_with("You ") {
        EverQuestLogKind::System
    } else {
        EverQuestLogKind::Other
    };
    event(
        timestamp,
        kind,
        None,
        None,
        None,
        None,
        compact_text(message),
    )
}

fn classify_outcome(
    timestamp: NaiveDateTime,
    timestamp_text: &str,
    message: &str,
) -> EverQuestCompactOutcome {
    if let Some(outcome) = classify_damage_outcome(timestamp, timestamp_text, message) {
        return outcome;
    }
    if let Some(outcome) = classify_spell_outcome(timestamp, timestamp_text, message) {
        return outcome;
    }
    if let Some(outcome) = classify_progress_outcome(timestamp, timestamp_text, message) {
        return outcome;
    }
    if let Some(outcome) = classify_rest_loot_death_outcome(timestamp, timestamp_text, message) {
        return outcome;
    }
    match parse_log_line(&format!("[{timestamp_text}] {message}")) {
        Ok(Some(event)) => outcome_from_log_event(event, timestamp_text),
        Ok(None) => unknown_outcome(timestamp, timestamp_text, "unclassified log line"),
        Err(error) => diagnostic_outcome(
            EverQuestOutcomeKind::Unknown,
            Some(timestamp_text),
            "parse_error",
            format!("log parser rejected line: {error}"),
        ),
    }
}

#[allow(clippy::too_many_lines)]
fn classify_damage_outcome(
    timestamp: NaiveDateTime,
    timestamp_text: &str,
    message: &str,
) -> Option<EverQuestCompactOutcome> {
    if let Some(captures) = dot_damage_regex().captures(message) {
        let actor = captures.name("actor").map(|value| value.as_str());
        let spell = captures.name("spell").map(|value| value.as_str());
        let amount = captures
            .name("amount")
            .and_then(|value| value.as_str().parse::<u32>().ok());
        return Some(outcome(
            timestamp,
            timestamp_text,
            if actor.is_some_and(|actor| actor.eq_ignore_ascii_case("you")) {
                EverQuestOutcomeKind::CombatDamageTaken
            } else {
                EverQuestOutcomeKind::HazardSignal
            },
            actor,
            None,
            spell,
            None,
            amount,
            None,
            None,
            None,
            format!(
                "{} took {} damage by {}",
                compact_text(actor.unwrap_or("unknown")),
                amount.map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                compact_text(spell.unwrap_or("unknown"))
            ),
            false,
            0.8,
            None,
        ));
    }
    if let Some(captures) = damage_taken_regex().captures(message) {
        let actor = captures.name("actor").map(|value| value.as_str());
        let amount = captures
            .name("amount")
            .and_then(|value| value.as_str().parse::<u32>().ok());
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::CombatDamageTaken,
            actor,
            Some("you"),
            None,
            None,
            amount,
            None,
            None,
            None,
            format!(
                "{} hit you for {} damage",
                compact_text(actor.unwrap_or("unknown")),
                amount.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
            ),
            false,
            0.85,
            None,
        ));
    }
    if let Some(captures) = damage_dealt_regex().captures(message) {
        let target = captures.name("target").map(|value| value.as_str());
        let amount = captures
            .name("amount")
            .and_then(|value| value.as_str().parse::<u32>().ok());
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::CombatDamageDealt,
            Some("you"),
            target,
            None,
            None,
            amount,
            None,
            None,
            None,
            format!(
                "you hit {} for {} damage",
                compact_text(target.unwrap_or("unknown")),
                amount.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
            ),
            false,
            0.85,
            None,
        ));
    }
    if message.contains("damage") || message.contains(" hits ") || message.contains(" hit ") {
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::AmbiguousCombat,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "ambiguous combat line",
            true,
            0.25,
            Some("ambiguous_combat"),
        ));
    }
    None
}

fn classify_spell_outcome(
    timestamp: NaiveDateTime,
    timestamp_text: &str,
    message: &str,
) -> Option<EverQuestCompactOutcome> {
    if let Some(captures) = spell_hit_regex().captures(message) {
        let spell = captures.name("spell").map(|value| value.as_str());
        let target = captures.name("target").map(|value| value.as_str());
        let amount = captures
            .name("amount")
            .and_then(|value| value.as_str().parse::<u32>().ok());
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::SpellHit,
            Some("you"),
            target,
            spell,
            None,
            amount,
            None,
            None,
            None,
            format!(
                "{} hit {} for {} damage",
                compact_text(spell.unwrap_or("spell")),
                compact_text(target.unwrap_or("unknown")),
                amount.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
            ),
            false,
            0.85,
            None,
        ));
    }
    if message == "Your spell fizzles!" {
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::SpellFizzle,
            Some("you"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "your spell fizzled",
            false,
            0.95,
            None,
        ));
    }
    if message.to_ascii_lowercase().contains("resist")
        && message.to_ascii_lowercase().contains("spell")
    {
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::SpellResist,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "spell resisted",
            true,
            0.65,
            Some("spell_resist_unparsed"),
        ));
    }
    None
}

fn classify_progress_outcome(
    timestamp: NaiveDateTime,
    timestamp_text: &str,
    message: &str,
) -> Option<EverQuestCompactOutcome> {
    if xp_gain_regex().is_match(message) {
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::XpGain,
            Some("you"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "experience gained",
            false,
            0.9,
            None,
        ));
    }
    if let Some(captures) = level_up_regex().captures(message) {
        let level = captures
            .name("level")
            .and_then(|value| value.as_str().parse::<u32>().ok());
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::LevelUp,
            Some("you"),
            None,
            None,
            None,
            None,
            level,
            None,
            None,
            format!(
                "level up to {}",
                level.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
            ),
            false,
            0.95,
            None,
        ));
    }
    None
}

fn classify_rest_loot_death_outcome(
    timestamp: NaiveDateTime,
    timestamp_text: &str,
    message: &str,
) -> Option<EverQuestCompactOutcome> {
    if let Some(captures) = death_regex().captures(message) {
        let actor = captures.name("actor").map(|value| value.as_str());
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::Death,
            Some("you"),
            actor,
            None,
            None,
            None,
            None,
            None,
            None,
            format!(
                "you were slain by {}",
                compact_text(actor.unwrap_or("unknown"))
            ),
            false,
            0.95,
            None,
        ));
    }
    if message.starts_with("LOADING, PLEASE WAIT")
        || message.starts_with("You regain consciousness")
    {
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::Respawn,
            Some("you"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "respawn or loading transition",
            true,
            0.55,
            Some("respawn_or_loading"),
        ));
    }
    if message == "You sit down."
        || message == "You begin to meditate."
        || message == "You stand up."
    {
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::RestSit,
            Some("you"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            if message == "You stand up." {
                "you stood"
            } else {
                "you rested"
            },
            false,
            0.9,
            None,
        ));
    }
    if message.starts_with("You receive ") || message.starts_with("You have looted ") {
        return Some(outcome(
            timestamp,
            timestamp_text,
            EverQuestOutcomeKind::Loot,
            Some("you"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "loot received",
            true,
            0.65,
            Some("loot_name_redacted"),
        ));
    }
    None
}

fn parse_location_message(
    timestamp: NaiveDateTime,
    message: &str,
) -> Result<EverQuestLogEvent, EverQuestLogError> {
    let rest = message
        .strip_prefix(location_prefix())
        .ok_or_else(|| EverQuestLogError::Location {
            message: message.to_owned(),
        })?
        .trim();
    let mut values = rest.split(',').map(str::trim);
    let display_y = parse_location_coord(values.next(), message)?;
    let display_x = parse_location_coord(values.next(), message)?;
    let display_z = parse_location_coord(values.next(), message)?;
    if values.next().is_some() {
        return Err(EverQuestLogError::Location {
            message: message.to_owned(),
        });
    }
    if !(display_y.is_finite() && display_x.is_finite() && display_z.is_finite()) {
        return Err(EverQuestLogError::Location {
            message: message.to_owned(),
        });
    }
    let location = EverQuestLocation {
        display_y,
        display_x,
        display_z,
    };
    Ok(EverQuestLogEvent {
        timestamp,
        kind: EverQuestLogKind::Location,
        actor: None,
        target: None,
        channel: None,
        level: None,
        zone: None,
        summary: format!(
            "location y={} x={} z={}",
            compact_coord(location.display_y),
            compact_coord(location.display_x),
            compact_coord(location.display_z)
        ),
        location: Some(location),
    })
}

fn parse_location_coord(value: Option<&str>, message: &str) -> Result<f64, EverQuestLogError> {
    let value =
        value
            .filter(|value| !value.is_empty())
            .ok_or_else(|| EverQuestLogError::Location {
                message: message.to_owned(),
            })?;
    value
        .parse::<f64>()
        .map_err(|_| EverQuestLogError::Location {
            message: message.to_owned(),
        })
}

fn parse_zone_entered_message(
    timestamp: NaiveDateTime,
    message: &str,
) -> Result<EverQuestLogEvent, EverQuestLogError> {
    let zone = message
        .strip_prefix(zone_entered_prefix())
        .unwrap_or_default()
        .trim()
        .trim_end_matches('.')
        .trim();
    if zone.is_empty() {
        return Err(EverQuestLogError::Zone {
            message: message.to_owned(),
        });
    }
    Ok(EverQuestLogEvent {
        timestamp,
        kind: EverQuestLogKind::ZoneEntered,
        actor: None,
        target: None,
        channel: None,
        level: None,
        location: None,
        zone: Some(zone.to_owned()),
        summary: format!("entered zone {zone}"),
    })
}

const fn zone_entered_prefix() -> &'static str {
    "You have entered "
}

fn classify_logging_or_target(
    timestamp: NaiveDateTime,
    message: &str,
) -> Option<EverQuestLogEvent> {
    if message.starts_with("Logging to ") {
        return Some(event(
            timestamp,
            EverQuestLogKind::LoggingEnabled,
            None,
            None,
            None,
            None,
            "logging enabled",
        ));
    }
    if let Some(target) = message.strip_prefix("Targeted (NPC): ") {
        return Some(event(
            timestamp,
            EverQuestLogKind::TargetNpc,
            None,
            Some(target),
            None,
            None,
            format!("target npc {}", compact_text(target)),
        ));
    }
    if let Some(target) = message.strip_prefix("Targeted (Player): ") {
        return Some(event(
            timestamp,
            EverQuestLogKind::TargetPlayer,
            None,
            Some(target),
            None,
            None,
            format!("target player {}", compact_text(target)),
        ));
    }
    if message == "You no longer have a target." {
        return Some(event(
            timestamp,
            EverQuestLogKind::TargetCleared,
            None,
            None,
            None,
            None,
            "target cleared",
        ));
    }
    None
}

fn classify_consider(timestamp: NaiveDateTime, message: &str) -> Option<EverQuestLogEvent> {
    let captures = consider_regex().captures(message)?;
    let target = captures.name("target").map(|value| value.as_str());
    let level = captures
        .name("level")
        .and_then(|value| value.as_str().parse::<u32>().ok());
    Some(event(
        timestamp,
        EverQuestLogKind::Consider,
        None,
        target,
        None,
        level,
        format!(
            "consider {} level {}",
            compact_text(target.unwrap_or("unknown")),
            level.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
        ),
    ))
}

fn classify_casting(timestamp: NaiveDateTime, message: &str) -> Option<EverQuestLogEvent> {
    if let Some(actor) = message.strip_suffix(" begins casting Gate.") {
        return Some(event(
            timestamp,
            EverQuestLogKind::CastBegins,
            Some(actor),
            None,
            None,
            None,
            format!("{} begins casting", compact_text(actor)),
        ));
    }
    if let Some(captures) = begins_casting_regex().captures(message) {
        let actor = captures.name("actor").map(|value| value.as_str());
        let spell = captures
            .name("spell")
            .map_or("spell", |value| value.as_str());
        return Some(event(
            timestamp,
            EverQuestLogKind::CastBegins,
            actor,
            None,
            None,
            None,
            format!(
                "{} begins casting {}",
                compact_text(actor.unwrap_or("unknown")),
                compact_text(spell)
            ),
        ));
    }
    if let Some(actor) = message.strip_suffix(" fades away.") {
        return Some(event(
            timestamp,
            EverQuestLogKind::CastResult,
            Some(actor),
            None,
            None,
            None,
            format!("{} fades away", compact_text(actor)),
        ));
    }
    None
}

fn classify_speech(timestamp: NaiveDateTime, message: &str) -> Option<EverQuestLogEvent> {
    if let Some(captures) = says_regex().captures(message) {
        let actor = captures.name("actor").map(|value| value.as_str());
        return Some(event(
            timestamp,
            EverQuestLogKind::Say,
            actor,
            None,
            None,
            None,
            format!("{} says", compact_text(actor.unwrap_or("unknown"))),
        ));
    }
    if let Some(captures) = tells_regex().captures(message) {
        let actor = captures.name("actor").map(|value| value.as_str());
        let channel = captures.name("channel").map(|value| value.as_str());
        return Some(event(
            timestamp,
            EverQuestLogKind::Tell,
            actor,
            None,
            channel,
            None,
            format!(
                "{} tells {}",
                compact_text(actor.unwrap_or("unknown")),
                compact_text(channel.unwrap_or("unknown"))
            ),
        ));
    }
    None
}

fn event(
    timestamp: NaiveDateTime,
    kind: EverQuestLogKind,
    actor: Option<&str>,
    target: Option<&str>,
    channel: Option<&str>,
    level: Option<u32>,
    summary: impl Into<String>,
) -> EverQuestLogEvent {
    EverQuestLogEvent {
        timestamp,
        kind,
        actor: actor.map(ToOwned::to_owned),
        target: target.map(ToOwned::to_owned),
        channel: channel.map(ToOwned::to_owned),
        level,
        location: None,
        zone: None,
        summary: summary.into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn outcome(
    timestamp: NaiveDateTime,
    timestamp_text: &str,
    kind: EverQuestOutcomeKind,
    actor: Option<&str>,
    target: Option<&str>,
    spell: Option<&str>,
    channel: Option<&str>,
    amount: Option<u32>,
    level: Option<u32>,
    zone: Option<&str>,
    location: Option<EverQuestLocation>,
    summary: impl Into<String>,
    redacted: bool,
    confidence: f32,
    diagnostic_code: Option<&str>,
) -> EverQuestCompactOutcome {
    EverQuestCompactOutcome {
        timestamp: Some(timestamp),
        timestamp_text: Some(timestamp_text.to_owned()),
        kind,
        actor: actor.map(compact_text),
        target: target.map(compact_text),
        spell: spell.map(compact_text),
        channel: channel.map(compact_text),
        amount,
        level,
        zone: zone.map(compact_text),
        location,
        summary: summary.into(),
        redacted,
        confidence,
        diagnostic_code: diagnostic_code.map(ToOwned::to_owned),
    }
}

fn diagnostic_outcome(
    kind: EverQuestOutcomeKind,
    timestamp_text: Option<&str>,
    code: &str,
    summary: impl Into<String>,
) -> EverQuestCompactOutcome {
    EverQuestCompactOutcome {
        timestamp: None,
        timestamp_text: timestamp_text.map(ToOwned::to_owned),
        kind,
        actor: None,
        target: None,
        spell: None,
        channel: None,
        amount: None,
        level: None,
        zone: None,
        location: None,
        summary: summary.into(),
        redacted: true,
        confidence: 0.0,
        diagnostic_code: Some(code.to_owned()),
    }
}

fn unknown_outcome(
    timestamp: NaiveDateTime,
    timestamp_text: &str,
    summary: impl Into<String>,
) -> EverQuestCompactOutcome {
    outcome(
        timestamp,
        timestamp_text,
        EverQuestOutcomeKind::Unknown,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        summary,
        true,
        0.1,
        Some("unknown"),
    )
}

fn outcome_from_log_event(
    event: EverQuestLogEvent,
    timestamp_text: &str,
) -> EverQuestCompactOutcome {
    let kind = match event.kind {
        EverQuestLogKind::Location => EverQuestOutcomeKind::Location,
        EverQuestLogKind::ZoneEntered => EverQuestOutcomeKind::ZoneEntered,
        EverQuestLogKind::TargetNpc => EverQuestOutcomeKind::TargetNpc,
        EverQuestLogKind::TargetPlayer => EverQuestOutcomeKind::TargetPlayer,
        EverQuestLogKind::TargetCleared => EverQuestOutcomeKind::TargetCleared,
        EverQuestLogKind::Consider => EverQuestOutcomeKind::Consider,
        EverQuestLogKind::CastBegins => EverQuestOutcomeKind::SpellBegins,
        EverQuestLogKind::Say | EverQuestLogKind::Tell => EverQuestOutcomeKind::ChatRedacted,
        EverQuestLogKind::LoggingEnabled
        | EverQuestLogKind::CastResult
        | EverQuestLogKind::System
        | EverQuestLogKind::Other => EverQuestOutcomeKind::Unknown,
    };
    let redacted = matches!(
        &kind,
        EverQuestOutcomeKind::ChatRedacted | EverQuestOutcomeKind::Unknown
    );
    let summary = if redacted && matches!(&kind, EverQuestOutcomeKind::ChatRedacted) {
        format!(
            "chat {}{}",
            event.channel.as_deref().map_or_else(
                || "message".to_owned(),
                |channel| format!("on {}", compact_text(channel))
            ),
            event
                .actor
                .as_deref()
                .map_or(String::new(), |actor| format!(
                    " from {}",
                    compact_text(actor)
                ))
        )
    } else if redacted {
        "unclassified log line".to_owned()
    } else {
        event.summary.clone()
    };
    EverQuestCompactOutcome {
        timestamp: Some(event.timestamp),
        timestamp_text: Some(timestamp_text.to_owned()),
        kind,
        actor: event.actor.map(|value| compact_text(&value)),
        target: event.target.map(|value| compact_text(&value)),
        spell: None,
        channel: event.channel.map(|value| compact_text(&value)),
        amount: None,
        level: event.level,
        zone: event.zone.map(|value| compact_text(&value)),
        location: event.location,
        summary,
        redacted,
        confidence: if redacted { 0.35 } else { 0.9 },
        diagnostic_code: redacted.then(|| "body_redacted_or_unclassified".to_owned()),
    }
}

const fn location_prefix() -> &'static str {
    "Your Location is"
}

fn compact_coord(value: f64) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn compact_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
    {
        if out.len() >= MAX_SUMMARY_CHARS {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

fn line_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^\[(?P<timestamp>[^\]]+)\]\s*(?P<message>.*)$")
            .unwrap_or_else(|error| panic!("EverQuest line regex invalid: {error}"))
    })
}

fn consider_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<target>.+) judges you .+ \(Lvl: (?P<level>[0-9]+)\)$")
            .unwrap_or_else(|error| panic!("EverQuest consider regex invalid: {error}"))
    })
}

fn begins_casting_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<actor>.+) begins casting (?P<spell>.+)\.$")
            .unwrap_or_else(|error| panic!("EverQuest cast regex invalid: {error}"))
    })
}

fn says_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<actor>.+) say(?:s)?, '.*'$")
            .unwrap_or_else(|error| panic!("EverQuest say regex invalid: {error}"))
    })
}

fn tells_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<actor>.+) tells (?P<channel>[^,]+), '.*'$")
            .unwrap_or_else(|error| panic!("EverQuest tell regex invalid: {error}"))
    })
}

fn damage_dealt_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^You (?:hit|slash|crush|pierce|kick|bash) (?P<target>.+) for (?P<amount>[0-9]+) points? of damage\.$")
            .unwrap_or_else(|error| panic!("EverQuest damage-dealt regex invalid: {error}"))
    })
}

fn damage_taken_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<actor>.+) (?:hits|hit|slashes|slash|crushes|crush|pierces|pierce|bites|bite|claws|claw|kicks|kick|bashes|bash) YOU for (?P<amount>[0-9]+) points? of damage\.$")
            .unwrap_or_else(|error| panic!("EverQuest damage-taken regex invalid: {error}"))
    })
}

fn dot_damage_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<actor>.+) has taken (?P<amount>[0-9]+) damage by (?P<spell>.+)\.$")
            .unwrap_or_else(|error| panic!("EverQuest dot-damage regex invalid: {error}"))
    })
}

fn spell_hit_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^Your (?P<spell>.+) (?:hits|hit) (?P<target>.+) for (?P<amount>[0-9]+) points? of .*damage\.$")
            .unwrap_or_else(|error| panic!("EverQuest spell-hit regex invalid: {error}"))
    })
}

fn xp_gain_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^You gain(?:ed)?(?: party)? experience!+$")
            .unwrap_or_else(|error| panic!("EverQuest xp regex invalid: {error}"))
    })
}

fn level_up_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^You have gained a level! Welcome to level (?P<level>[0-9]+)!$")
            .unwrap_or_else(|error| panic!("EverQuest level-up regex invalid: {error}"))
    })
}

fn death_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^You have been slain by (?P<actor>.+)!$")
            .unwrap_or_else(|error| panic!("EverQuest death regex invalid: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_filename_identity() {
        let identity = parse_log_file_name("eqlog_Thenumberone_frostreaver.txt")
            .unwrap_or_else(|| panic!("expected identity"));
        assert_eq!(identity.character, "Thenumberone");
        assert_eq!(identity.server, "frostreaver");
    }

    #[test]
    fn parses_target_and_consider_events() -> Result<(), EverQuestLogError> {
        let target = parse_log_line("[Thu May 28 06:48:10 2026] Targeted (NPC): Olavn N`Mar")?
            .unwrap_or_else(|| panic!("expected target event"));
        assert_eq!(target.kind, EverQuestLogKind::TargetNpc);
        assert_eq!(target.target.as_deref(), Some("Olavn N`Mar"));

        let consider = parse_log_line(
            "[Thu May 28 06:45:29 2026] Camia V`Retta judges you amiably -- what would you like your tombstone to say? (Lvl: 70)",
        )?
        .unwrap_or_else(|| panic!("expected consider event"));
        assert_eq!(consider.kind, EverQuestLogKind::Consider);
        assert_eq!(consider.target.as_deref(), Some("Camia V`Retta"));
        assert_eq!(consider.level, Some(70));
        Ok(())
    }

    #[test]
    fn parses_location_event_in_display_order() -> Result<(), EverQuestLogError> {
        let event =
            parse_log_line("[Thu May 28 11:00:00 2026] Your Location is -14.50, 23.25, 7.00")?
                .unwrap_or_else(|| panic!("expected location event"));
        assert_eq!(event.kind, EverQuestLogKind::Location);
        let location = event
            .location
            .unwrap_or_else(|| panic!("expected location payload"));
        assert!((location.display_y - -14.5).abs() < f64::EPSILON);
        assert!((location.display_x - 23.25).abs() < f64::EPSILON);
        assert!((location.display_z - 7.0).abs() < f64::EPSILON);
        assert_eq!(event.summary, "location y=-14.5 x=23.25 z=7");
        Ok(())
    }

    #[test]
    fn parses_zone_entered_event() -> Result<(), EverQuestLogError> {
        let event = parse_log_line(
            "[Thu May 28 12:45:46 2026] You have entered Neriak - Foreign Quarter.",
        )?
        .unwrap_or_else(|| panic!("expected zone event"));
        assert_eq!(event.kind, EverQuestLogKind::ZoneEntered);
        assert_eq!(event.zone.as_deref(), Some("Neriak - Foreign Quarter"));
        assert_eq!(event.summary, "entered zone Neriak - Foreign Quarter");
        Ok(())
    }

    #[test]
    fn malformed_location_event_fails_closed() {
        let error =
            match parse_log_line("[Thu May 28 11:00:00 2026] Your Location is 1.0, nope, 3.0") {
                Ok(value) => panic!("malformed location parsed unexpectedly: {value:?}"),
                Err(error) => error,
            };
        assert!(matches!(error, EverQuestLogError::Location { .. }));
    }

    #[test]
    fn token_summary_suppresses_chat_body() -> Result<(), EverQuestLogError> {
        let event = parse_log_line(
            "[Thu May 28 06:48:08 2026] Mikaylah tells general3:2, 'long player chat text that should not be copied into the compact summary'",
        )?
        .unwrap_or_else(|| panic!("expected tell event"));
        assert_eq!(event.kind, EverQuestLogKind::Tell);
        assert_eq!(event.actor.as_deref(), Some("Mikaylah"));
        assert_eq!(event.channel.as_deref(), Some("general3:2"));
        assert_eq!(event.summary, "Mikaylah tells general3:2");
        Ok(())
    }

    #[test]
    fn player_say_variant_is_redacted_chat() -> Result<(), EverQuestLogError> {
        let event = parse_log_line("[Thu May 28 11:00:00 2026] You say, '/loc'")?
            .unwrap_or_else(|| panic!("expected player say event"));
        assert_eq!(event.kind, EverQuestLogKind::Say);
        assert_eq!(event.actor.as_deref(), Some("You"));
        assert_eq!(event.summary, "You says");
        Ok(())
    }

    #[test]
    fn compact_outcome_classifies_damage_and_hazard() {
        let outcome = parse_outcome_line(
            "[Thu May 28 15:55:58 2026] Kimmuriel has taken 1 damage by Rabies.",
        );
        assert_eq!(outcome.kind, EverQuestOutcomeKind::HazardSignal);
        assert_eq!(outcome.actor.as_deref(), Some("Kimmuriel"));
        assert_eq!(outcome.spell.as_deref(), Some("Rabies"));
        assert_eq!(outcome.amount, Some(1));
        assert!(!outcome.redacted);
    }

    #[test]
    fn compact_outcome_redacts_chat_body() {
        let outcome = parse_outcome_line(
            "[Thu May 28 15:52:42 2026] Donafu tells NewPlayers3:1, 'thats me. ill be tanking the floor'",
        );
        assert_eq!(outcome.kind, EverQuestOutcomeKind::ChatRedacted);
        assert_eq!(outcome.actor.as_deref(), Some("Donafu"));
        assert_eq!(outcome.channel.as_deref(), Some("NewPlayers3:1"));
        assert!(outcome.redacted);
        assert!(!outcome.summary.contains("tanking"));
    }

    #[test]
    fn compact_outcome_malformed_timestamp_becomes_diagnostic() {
        let outcome = parse_outcome_line("[Bad Time] You gain experience!!");
        assert_eq!(
            outcome.kind,
            EverQuestOutcomeKind::DiagnosticMalformedTimestamp
        );
        assert_eq!(
            outcome.diagnostic_code.as_deref(),
            Some("malformed_timestamp")
        );
        assert!(outcome.redacted);
    }

    #[test]
    fn compact_outcome_ambiguous_combat_fails_to_low_confidence() {
        let outcome = parse_outcome_line("[Thu May 28 11:00:00 2026] Something hits for damage.");
        assert_eq!(outcome.kind, EverQuestOutcomeKind::AmbiguousCombat);
        assert!(outcome.confidence < 0.5);
        assert_eq!(outcome.diagnostic_code.as_deref(), Some("ambiguous_combat"));
    }
}
