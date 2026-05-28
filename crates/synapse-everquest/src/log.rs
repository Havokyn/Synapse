use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use chrono::NaiveDateTime;
use regex::Regex;
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EverQuestLogEvent {
    pub timestamp: NaiveDateTime,
    pub kind: EverQuestLogKind,
    pub actor: Option<String>,
    pub target: Option<String>,
    pub channel: Option<String>,
    pub level: Option<u32>,
    pub summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
    Ok(Some(classify_event(timestamp, message)))
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
        summary: summary.into(),
    }
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
        Regex::new(r"^(?P<actor>.+) says, '.*'$")
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
}
