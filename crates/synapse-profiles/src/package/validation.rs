use std::{collections::HashSet, path::Path};

use chrono::DateTime;
use regex::Regex;
use synapse_core::{ProfileUseScope, SCHEMA_VERSION};

use super::types::{
    DisplayAssumptions, PackageAssumptions, PackageAuthor, PackageChangelogEntry,
    PackageDependency, PackageFiles, PackageHashes, PackageInput, PackagePermissions,
    PackageSignature, PackageSource, PackageTarget, PackageTrust, ProfilePackageManifest,
};
use crate::error::ProfileError;

pub const PROFILE_PACKAGE_SCHEMA_VERSION: u32 = 1;
pub const PROFILE_PACKAGE_KIND: &str = "profile_package";

const APPROVED_PROFILE_LICENSES: [&str; 3] = ["MIT", "Apache-2.0", "MIT OR Apache-2.0"];
const METADATA_POISON_MARKERS: &[&str] = &[
    "ignore previous instructions",
    "ignore all previous instructions",
    "disregard previous instructions",
    "system prompt",
    "developer message",
    "tool call",
    "exfiltrate",
    "reveal secrets",
    "jailbreak",
    "prompt injection",
    "run powershell",
    "execute shell",
    "download and run",
    "act_run_shell",
];

impl ProfilePackageManifest {
    /// Validates this manifest using the current local package policy.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] when a field is missing, ambiguous, unsafe, or
    /// incompatible with the current Synapse schema.
    pub fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        if self.schema_version > PROFILE_PACKAGE_SCHEMA_VERSION {
            return Err(ProfileError::VersionIncompatible {
                path: path.to_path_buf(),
                schema_version: self.schema_version,
                supported_version: PROFILE_PACKAGE_SCHEMA_VERSION,
            });
        }
        require_equals(path, "kind", &self.kind, PROFILE_PACKAGE_KIND)?;
        validate_package_id(path, "package_id", &self.package_id)?;
        validate_package_id(path, "profile_id", &self.profile_id)?;
        validate_semver(path, "package_version", &self.package_version)?;
        validate_semver(path, "profile_version", &self.profile_version)?;
        validate_timestamp(path, "created_at", &self.created_at)?;
        self.author.validate(path)?;
        self.source.validate(path)?;
        validate_targets(path, &self.targets)?;
        self.assumptions.validate(path)?;
        self.input.validate(path)?;
        self.permissions.validate(path)?;
        validate_changelog(path, &self.changelog)?;
        self.hashes.validate(path)?;
        self.files.validate(path)?;
        self.trust.validate(path)?;
        validate_signatures(path, &self.signatures)?;
        validate_manifest_text_safety(path, self)?;
        if self.trust.policy == "signed_required" && self.signatures.is_empty() {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: "trust.policy signed_required requires at least one package signature"
                    .to_owned(),
            });
        }
        Ok(())
    }
}

impl PackageAuthor {
    fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        require_non_empty(path, "author.name", &self.name)?;
        require_non_empty(path, "author.contact", &self.contact)?;
        require_non_empty(path, "author.attribution", &self.attribution)
    }
}

impl PackageSource {
    fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        require_in_set(
            path,
            "source.kind",
            &self.kind,
            &["bundled", "local_user", "registry", "synthetic_fixture"],
        )?;
        require_non_empty(path, "source.uri", &self.uri)?;
        require_non_empty(path, "source.revision", &self.revision)?;
        require_non_empty(path, "source.built_by", &self.built_by)?;
        require_non_empty(path, "source.generated_by", &self.generated_by)
    }
}

impl PackageAssumptions {
    fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        require_in_set(path, "assumptions.os", &self.os, &["windows"])?;
        if self.synapse_schema_version != SCHEMA_VERSION {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: format!(
                    "assumptions.synapse_schema_version must be {SCHEMA_VERSION}; got {}",
                    self.synapse_schema_version
                ),
            });
        }
        self.display.validate(path)
    }
}

impl DisplayAssumptions {
    fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        if self.min_width == 0 || self.min_height == 0 {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: "display min_width and min_height must be positive".to_owned(),
            });
        }
        if self.dpi_scale_min <= 0.0
            || self.dpi_scale_max <= 0.0
            || self.dpi_scale_min > self.dpi_scale_max
        {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: format!(
                    "display DPI range must be positive and ordered; got {}..{}",
                    self.dpi_scale_min, self.dpi_scale_max
                ),
            });
        }
        Ok(())
    }
}

impl PackageInput {
    fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        if self.backends.is_empty() {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: "input.backends must include at least one backend".to_owned(),
            });
        }
        let mut seen = HashSet::new();
        for backend in &self.backends {
            if !seen.insert(*backend) {
                return Err(ProfileError::Parse {
                    path: path.to_path_buf(),
                    message: format!("duplicate input backend {backend:?}"),
                });
            }
        }
        for dependency in self.firmware.iter().chain(&self.models) {
            dependency.validate(path)?;
        }
        Ok(())
    }
}

impl PackageDependency {
    fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        require_non_empty(path, "dependency.id", &self.id)?;
        validate_semver(path, "dependency.version", &self.version)?;
        if let Some(digest) = &self.digest {
            validate_sha256_digest(path, "dependency.digest", digest)?;
        }
        Ok(())
    }
}

impl PackagePermissions {
    fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        validate_license(path, &self.license_spdx)?;
        require_in_set(
            path,
            "permissions.contribution_terms",
            &self.contribution_terms,
            &["DCO-1.1", "none"],
        )?;
        if self.use_scope == ProfileUseScope::Unknown {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: "permissions.use_scope must not be unknown for an installable package"
                    .to_owned(),
            });
        }
        if self.contribution.share_audit_allowed && !self.contribution.export_allowed {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: "permissions.contribution.share_audit_allowed requires export_allowed"
                    .to_owned(),
            });
        }
        if !self.execution.local_only || self.execution.remote_server_allowed {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: "permissions.execution must be local_only=true and remote_server_allowed=false for installable packages".to_owned(),
            });
        }
        Ok(())
    }
}

impl PackageHashes {
    fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        validate_sha256_digest(
            path,
            "hashes.profile_toml_sha256",
            &self.profile_toml_sha256,
        )?;
        validate_sha256_digest(path, "hashes.package_sha256", &self.package_sha256)?;
        for (name, digest) in &self.assets {
            require_non_empty(path, "hashes.assets key", name)?;
            validate_sha256_digest(path, "hashes.assets value", digest)?;
        }
        Ok(())
    }
}

impl PackageFiles {
    fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        require_non_empty(path, "files.profile_toml", &self.profile_toml)?;
        for asset in &self.assets {
            require_non_empty(path, "files.assets entry", asset)?;
        }
        Ok(())
    }
}

impl PackageTrust {
    fn validate(&self, path: &Path) -> Result<(), ProfileError> {
        require_in_set(
            path,
            "trust.policy",
            &self.policy,
            &["local_unsigned_allowed", "signed_required"],
        )?;
        let mut seen = HashSet::new();
        for signer in &self.required_signers {
            validate_package_id(path, "trust.required_signers", signer)?;
            if !seen.insert(signer) {
                return Err(ProfileError::Parse {
                    path: path.to_path_buf(),
                    message: format!("duplicate trust.required_signers entry {signer:?}"),
                });
            }
        }
        Ok(())
    }
}

fn validate_signatures(path: &Path, signatures: &[PackageSignature]) -> Result<(), ProfileError> {
    let mut seen = HashSet::new();
    for signature in signatures {
        validate_package_id(path, "signature.signer_id", &signature.signer_id)?;
        validate_sha256_digest(path, "signature.key_id", &signature.key_id)?;
        require_in_set(
            path,
            "signature.algorithm",
            &signature.algorithm,
            &["ed25519"],
        )?;
        validate_ed25519_signature(path, "signature.signature", &signature.signature)?;
        let unique = format!(
            "{}:{}:{}",
            signature.signer_id, signature.key_id, signature.algorithm
        );
        if !seen.insert(unique) {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: format!(
                    "duplicate package signature for signer {} and key {}",
                    signature.signer_id, signature.key_id
                ),
            });
        }
    }
    Ok(())
}

fn validate_targets(path: &Path, targets: &[PackageTarget]) -> Result<(), ProfileError> {
    if targets.is_empty() {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: "targets must include at least one compatibility target".to_owned(),
        });
    }
    for target in targets {
        require_non_empty(path, "target.target_id", &target.target_id)?;
        require_non_empty(path, "target.target_kind", &target.target_kind)?;
        if target.app_id.is_none()
            && target.process_name.is_none()
            && target.title_regex.is_none()
            && target.steam_appid.is_none()
        {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: format!(
                    "target {:?} must define app_id, process_name, title_regex, or steam_appid",
                    target.target_id
                ),
            });
        }
        if let Some(pattern) = &target.title_regex {
            Regex::new(pattern).map_err(|source| ProfileError::Parse {
                path: path.to_path_buf(),
                message: format!("invalid target title_regex {pattern:?}: {source}"),
            })?;
        }
        if let Some(version) = &target.app_version {
            require_non_empty(path, "target.app_version", version)?;
        }
    }
    Ok(())
}

fn validate_changelog(
    path: &Path,
    changelog: &[PackageChangelogEntry],
) -> Result<(), ProfileError> {
    if changelog.is_empty() {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: "changelog must include at least one entry".to_owned(),
        });
    }
    for entry in changelog {
        validate_semver(path, "changelog.version", &entry.version)?;
        validate_timestamp(path, "changelog.at", &entry.at)?;
        require_non_empty(path, "changelog.summary", &entry.summary)?;
    }
    Ok(())
}

fn validate_manifest_text_safety(
    path: &Path,
    manifest: &ProfilePackageManifest,
) -> Result<(), ProfileError> {
    validate_text_field_safe(path, "author.name", &manifest.author.name)?;
    validate_text_field_safe(path, "author.contact", &manifest.author.contact)?;
    validate_text_field_safe(path, "author.attribution", &manifest.author.attribution)?;
    validate_text_field_safe(path, "source.uri", &manifest.source.uri)?;
    validate_text_field_safe(path, "source.revision", &manifest.source.revision)?;
    validate_text_field_safe(path, "source.built_by", &manifest.source.built_by)?;
    validate_text_field_safe(path, "source.generated_by", &manifest.source.generated_by)?;
    for (index, entry) in manifest.changelog.iter().enumerate() {
        validate_text_field_safe(path, &format!("changelog[{index}].summary"), &entry.summary)?;
    }
    for (key, value) in &manifest.metadata {
        validate_text_field_safe(path, "metadata key", key)?;
        validate_text_field_safe(path, &format!("metadata.{key}"), value)?;
    }
    Ok(())
}

fn validate_text_field_safe(path: &Path, field: &str, value: &str) -> Result<(), ProfileError> {
    let normalized = value.to_ascii_lowercase();
    if let Some(marker) = METADATA_POISON_MARKERS
        .iter()
        .find(|marker| normalized.contains(**marker))
    {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!("{field} contains unsafe metadata instruction marker {marker:?}"),
        });
    }
    Ok(())
}

fn validate_package_id(path: &Path, field: &str, value: &str) -> Result<(), ProfileError> {
    require_non_empty(path, field, value)?;
    if !value.contains('.') {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!("{field} must contain a registry or namespace separator '.'"),
        });
    }
    if !value.chars().all(|item| {
        item.is_ascii_lowercase() || item.is_ascii_digit() || matches!(item, '.' | '-' | '_')
    }) {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!("{field} must use lowercase ascii letters, digits, '.', '-', or '_'"),
        });
    }
    Ok(())
}

fn validate_semver(path: &Path, field: &str, value: &str) -> Result<(), ProfileError> {
    require_non_empty(path, field, value)?;
    let (without_build, build) = split_once_optional(value, '+', path, field)?;
    if let Some(build) = build {
        validate_identifier_list(path, field, build, false)?;
    }
    let (core, prerelease) = split_once_optional(without_build, '-', path, field)?;
    let parts = core.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!("{field} must be semantic version major.minor.patch"),
        });
    }
    for part in parts {
        validate_numeric_identifier(path, field, part)?;
    }
    if let Some(prerelease) = prerelease {
        validate_identifier_list(path, field, prerelease, true)?;
    }
    Ok(())
}

fn split_once_optional<'a>(
    value: &'a str,
    separator: char,
    path: &Path,
    field: &str,
) -> Result<(&'a str, Option<&'a str>), ProfileError> {
    let parts = value.split(separator).collect::<Vec<_>>();
    match parts.as_slice() {
        [head] => Ok((head, None)),
        [head, tail] if !head.is_empty() && !tail.is_empty() => Ok((head, Some(tail))),
        _ => Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!("{field} has invalid semantic-version separator placement"),
        }),
    }
}

fn validate_numeric_identifier(path: &Path, field: &str, value: &str) -> Result<(), ProfileError> {
    if value.is_empty()
        || !value.chars().all(|item| item.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!(
                "{field} numeric identifiers must be non-negative with no leading zero"
            ),
        });
    }
    Ok(())
}

fn validate_identifier_list(
    path: &Path,
    field: &str,
    value: &str,
    reject_numeric_leading_zero: bool,
) -> Result<(), ProfileError> {
    for item in value.split('.') {
        if item.is_empty()
            || !item
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            return Err(ProfileError::Parse {
                path: path.to_path_buf(),
                message: format!("{field} contains invalid semantic-version identifier {item:?}"),
            });
        }
        if reject_numeric_leading_zero && item.chars().all(|character| character.is_ascii_digit()) {
            validate_numeric_identifier(path, field, item)?;
        }
    }
    Ok(())
}

fn validate_timestamp(path: &Path, field: &str, value: &str) -> Result<(), ProfileError> {
    require_non_empty(path, field, value)?;
    DateTime::parse_from_rfc3339(value).map_err(|source| ProfileError::Parse {
        path: path.to_path_buf(),
        message: format!("{field} must be RFC3339: {source}"),
    })?;
    Ok(())
}

fn validate_license(path: &Path, value: &str) -> Result<(), ProfileError> {
    require_non_empty(path, "permissions.license_spdx", value)?;
    if !APPROVED_PROFILE_LICENSES.contains(&value) {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!("permissions.license_spdx {value:?} is not approved for profiles"),
        });
    }
    Ok(())
}

pub fn validate_sha256_digest(path: &Path, field: &str, value: &str) -> Result<(), ProfileError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!("{field} must start with sha256:"),
        });
    };
    if hex.len() != 64 || !hex.chars().all(|item| item.is_ascii_hexdigit()) {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!("{field} must contain a 64-hex SHA-256 digest"),
        });
    }
    Ok(())
}

fn validate_ed25519_signature(path: &Path, field: &str, value: &str) -> Result<(), ProfileError> {
    let Some(hex) = value.strip_prefix("ed25519:") else {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!("{field} must start with ed25519:"),
        });
    };
    if hex.len() != 128 || !hex.chars().all(|item| item.is_ascii_hexdigit()) {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!("{field} must contain a 128-hex Ed25519 signature"),
        });
    }
    Ok(())
}

fn require_equals(
    path: &Path,
    field: &str,
    value: &str,
    expected: &str,
) -> Result<(), ProfileError> {
    if value == expected {
        return Ok(());
    }
    Err(ProfileError::Parse {
        path: path.to_path_buf(),
        message: format!("{field} must be {expected:?}; got {value:?}"),
    })
}

fn require_in_set(
    path: &Path,
    field: &str,
    value: &str,
    allowed: &[&str],
) -> Result<(), ProfileError> {
    if allowed.contains(&value) {
        return Ok(());
    }
    Err(ProfileError::Parse {
        path: path.to_path_buf(),
        message: format!("{field} has unsupported value {value:?}"),
    })
}

fn require_non_empty(path: &Path, field: &str, value: &str) -> Result<(), ProfileError> {
    if !value.trim().is_empty() {
        return Ok(());
    }
    Err(ProfileError::Parse {
        path: path.to_path_buf(),
        message: format!("{field} must not be empty"),
    })
}
