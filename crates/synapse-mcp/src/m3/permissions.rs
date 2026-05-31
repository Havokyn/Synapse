use std::{
    collections::BTreeSet,
    fmt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use rmcp::{ErrorData, model::ErrorCode};
use serde_json::json;
use synapse_core::{Action, Backend, ComboInput, ComboStep, error_codes};

pub type RequiredPermissions = BTreeSet<Permission>;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Permission {
    ReadEvents,
    WriteReflex,
    ReadReflex,
    ReadProfile,
    WriteProfileActive,
    WriteReplay,
    ReadAudio,
    ReadStorage,
    WriteStorage,
    InputKeyboard,
    InputMouse,
    InputPad,
    InputHardwareHid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionGrants {
    granted: RequiredPermissions,
}

impl PermissionGrants {
    pub fn from_config(raw: Option<&str>, audio_enabled: bool) -> Result<Self> {
        let granted = match raw {
            Some(raw) => parse_grants(raw)?,
            None => default_grants(audio_enabled),
        };
        if granted.contains(&Permission::ReadAudio) && !audio_enabled {
            bail!("READ_AUDIO requires --enable-audio or SYNAPSE_ENABLE_AUDIO=true");
        }
        Ok(Self { granted })
    }

    #[must_use]
    pub fn first_missing(&self, required: &RequiredPermissions) -> Option<Permission> {
        required
            .iter()
            .find(|permission| !self.granted.contains(permission))
            .copied()
    }

    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.granted
            .iter()
            .map(|permission| permission.as_str())
            .collect()
    }
}

impl Permission {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadEvents => "READ_EVENTS",
            Self::WriteReflex => "WRITE_REFLEX",
            Self::ReadReflex => "READ_REFLEX",
            Self::ReadProfile => "READ_PROFILE",
            Self::WriteProfileActive => "WRITE_PROFILE_ACTIVE",
            Self::WriteReplay => "WRITE_REPLAY",
            Self::ReadAudio => "READ_AUDIO",
            Self::ReadStorage => "READ_STORAGE",
            Self::WriteStorage => "WRITE_STORAGE",
            Self::InputKeyboard => "INPUT_KEYBOARD",
            Self::InputMouse => "INPUT_MOUSE",
            Self::InputPad => "INPUT_PAD",
            Self::InputHardwareHid => "INPUT_HARDWARE_HID",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        let normalized = raw.trim().replace(['-', ' '], "_").to_ascii_uppercase();
        match normalized.as_str() {
            "READ_EVENTS" => Ok(Self::ReadEvents),
            "WRITE_REFLEX" => Ok(Self::WriteReflex),
            "READ_REFLEX" => Ok(Self::ReadReflex),
            "READ_PROFILE" => Ok(Self::ReadProfile),
            "WRITE_PROFILE_ACTIVE" => Ok(Self::WriteProfileActive),
            "WRITE_REPLAY" => Ok(Self::WriteReplay),
            "READ_AUDIO" => Ok(Self::ReadAudio),
            "READ_STORAGE" => Ok(Self::ReadStorage),
            "WRITE_STORAGE" => Ok(Self::WriteStorage),
            "INPUT_KEYBOARD" | "KEYBOARD" => Ok(Self::InputKeyboard),
            "INPUT_MOUSE" | "MOUSE" => Ok(Self::InputMouse),
            "INPUT_PAD" | "PAD" => Ok(Self::InputPad),
            "INPUT_HARDWARE_HID" | "HARDWARE_HID" => Ok(Self::InputHardwareHid),
            other => bail!("unknown M3 permission {other:?}"),
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[must_use]
pub fn required<const N: usize>(permissions: [Permission; N]) -> RequiredPermissions {
    permissions.into_iter().collect()
}

pub fn authorization_error(tool: &str, missing: Permission) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!("tool {tool} requires permission {missing}"),
        Some(json!({
            "code": error_codes::SAFETY_PERMISSION_DENIED,
            "tool": tool,
            "missing_permission": missing.as_str(),
        })),
    )
}

pub fn profile_scope_error(profile_id: &str) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "profile {profile_id} has use_scope=\"unknown\"; start with --allow-unknown-profile to activate it"
        ),
        Some(json!({
            "code": error_codes::SAFETY_PROFILE_ACTION_DENIED,
            "profile_id": profile_id,
            "use_scope": "unknown",
        })),
    )
}

pub fn replay_path_error(path: &Path, root: &Path) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "replay_record path {} must stay under {}",
            path.display(),
            root.display()
        ),
        Some(json!({
            "code": error_codes::SAFETY_PERMISSION_DENIED,
            "permission": Permission::WriteReplay.as_str(),
            "reason": "path_outside_allow_root",
            "path": path.display().to_string(),
            "allow_root": root.display().to_string(),
        })),
    )
}

pub fn add_action_permissions(action: &Action, required: &mut RequiredPermissions) {
    match action {
        Action::KeyPress { backend, .. }
        | Action::KeyDown { backend, .. }
        | Action::KeyUp { backend, .. }
        | Action::KeyChord { backend, .. }
        | Action::TypeText { backend, .. } => {
            required.insert(Permission::InputKeyboard);
            add_backend_permission(*backend, required);
        }
        Action::MouseMove { backend, .. }
        | Action::MouseMoveRelative { backend, .. }
        | Action::MouseButton { backend, .. }
        | Action::MouseDrag { backend, .. }
        | Action::MouseScroll { backend, .. }
        | Action::AimAt { backend, .. } => {
            required.insert(Permission::InputMouse);
            add_backend_permission(*backend, required);
        }
        Action::PadButton { .. }
        | Action::PadStick { .. }
        | Action::PadTrigger { .. }
        | Action::PadReport { .. } => {
            required.insert(Permission::InputPad);
        }
        Action::Combo { steps, backend } => {
            add_backend_permission(*backend, required);
            add_combo_step_permissions(steps, *backend, required);
        }
        Action::ReleaseAll => {
            required.insert(Permission::InputKeyboard);
            required.insert(Permission::InputMouse);
            required.insert(Permission::InputPad);
        }
    }
}

pub fn normalize_replay_path(
    root: &Path,
    path: Option<&str>,
) -> std::result::Result<PathBuf, ErrorData> {
    let Some(raw_path) = path.map(str::trim) else {
        return Ok(default_replay_path(root));
    };
    if raw_path.is_empty() {
        return Err(crate::m1::mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "replay_record path must not be empty when provided",
        ));
    }

    let requested = PathBuf::from(raw_path);
    let candidate = if requested.is_absolute() {
        requested
    } else {
        root.join(requested)
    };
    let root = lexical_normalize(root);
    let candidate = lexical_normalize(&candidate);
    if is_under_root(&candidate, &root) {
        Ok(candidate)
    } else {
        Err(replay_path_error(&candidate, &root))
    }
}

#[must_use]
pub fn replay_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("synapse")
        .join("replays")
}

fn parse_grants(raw: &str) -> Result<RequiredPermissions> {
    let trimmed = raw.trim();
    if matches!(
        trimmed.to_ascii_uppercase().as_str(),
        "" | "NONE" | "DENY_ALL"
    ) {
        return Ok(RequiredPermissions::new());
    }
    let mut granted = RequiredPermissions::new();
    for token in trimmed
        .split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace())
        .filter(|token| !token.trim().is_empty())
    {
        granted.insert(Permission::parse(token)?);
    }
    Ok(granted)
}

fn default_grants(audio_enabled: bool) -> RequiredPermissions {
    // Permissive by default: every non-audio permission is granted so a stock
    // daemon can fully drive the Windows desktop. READ_AUDIO is a real
    // capability dependency (requires --enable-audio / SYNAPSE_ENABLE_AUDIO),
    // not a guardrail, so it stays conditional. INPUT_HARDWARE_HID is granted
    // here but still requires an attached Pico HID device to actually emit.
    let mut granted = required([
        Permission::ReadEvents,
        Permission::WriteReflex,
        Permission::ReadReflex,
        Permission::ReadProfile,
        Permission::WriteProfileActive,
        Permission::WriteReplay,
        Permission::ReadStorage,
        Permission::WriteStorage,
        Permission::InputKeyboard,
        Permission::InputMouse,
        Permission::InputPad,
        Permission::InputHardwareHid,
    ]);
    if audio_enabled {
        granted.insert(Permission::ReadAudio);
    }
    granted
}

fn add_backend_permission(backend: Backend, required: &mut RequiredPermissions) {
    if backend == Backend::Hardware {
        required.insert(Permission::InputHardwareHid);
    }
}

fn add_combo_step_permissions(
    steps: &[ComboStep],
    backend: Backend,
    required: &mut RequiredPermissions,
) {
    for step in steps {
        match step.input {
            ComboInput::KeyDown { .. } | ComboInput::KeyUp { .. } | ComboInput::KeyPress { .. } => {
                required.insert(Permission::InputKeyboard);
                add_backend_permission(backend, required);
            }
            ComboInput::MouseButton { .. } | ComboInput::MouseMoveRel { .. } => {
                required.insert(Permission::InputMouse);
                add_backend_permission(backend, required);
            }
            ComboInput::PadButton { .. } | ComboInput::PadStick { .. } => {
                required.insert(Permission::InputPad);
            }
        }
    }
}

fn default_replay_path(root: &Path) -> PathBuf {
    root.join(format!("replay-{}.jsonl", synapse_core::new_session_id()))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy().replace('\\', "/");
    let (prefix, rest) = split_windows_drive_prefix(&raw);
    let rooted = rest.starts_with('/');
    let mut parts = Vec::new();

    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !rooted && prefix.is_empty() {
                    parts.push(part);
                }
            }
            _ => parts.push(part),
        }
    }

    let separator = if prefix.is_empty() { "/" } else { "\\" };
    let mut normalized = String::new();
    if !prefix.is_empty() {
        normalized.push_str(prefix);
        if rooted {
            normalized.push('\\');
        }
    } else if rooted {
        normalized.push('/');
    }
    normalized.push_str(&parts.join(separator));
    PathBuf::from(normalized)
}

fn split_windows_drive_prefix(raw: &str) -> (&str, &str) {
    if raw.len() >= 2 && raw.as_bytes()[1] == b':' && raw.as_bytes()[0].is_ascii_alphabetic() {
        raw.split_at(2)
    } else {
        ("", raw)
    }
}

fn is_under_root(path: &Path, root: &Path) -> bool {
    let path_key = comparable_path(path);
    let root_key = comparable_path(root);
    path_key == root_key
        || path_key
            .strip_prefix(&root_key)
            .is_some_and(|suffix| suffix.starts_with(['\\', '/']))
}

fn comparable_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

pub fn configured_grants_from_parts(
    raw: Option<&str>,
    audio_enabled: bool,
) -> Result<PermissionGrants> {
    PermissionGrants::from_config(raw, audio_enabled).context("parse M3 permission grants")
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "unit tests intentionally keep failure messages close to the assertion"
)]
mod tests {
    use super::*;

    #[test]
    fn explicit_read_audio_grant_requires_audio_flag() {
        assert!(PermissionGrants::from_config(Some("READ_AUDIO"), false).is_err());
        let grants = PermissionGrants::from_config(Some("READ_AUDIO"), true)
            .expect("READ_AUDIO should parse when audio is enabled");
        assert_eq!(
            grants.first_missing(&required([Permission::ReadAudio])),
            None
        );
    }

    #[test]
    fn replay_path_must_stay_under_root_after_parent_components() {
        let root = PathBuf::from(r"C:\Users\hotra\AppData\Local\synapse\replays");
        let inside = normalize_replay_path(&root, Some("ok\\demo.jsonl"))
            .expect("relative path should resolve under root");
        assert!(is_under_root(&inside, &root));

        let outside = normalize_replay_path(&root, Some("..\\outside.jsonl"))
            .expect_err("parent traversal must leave the allow root");
        assert_eq!(
            outside.data.as_ref().and_then(|data| data.get("code")),
            Some(&json!(error_codes::SAFETY_PERMISSION_DENIED))
        );
    }
}
