# 22 - Local Profile Registry Data Model

## 1. Status and authority

This document is the local data-model baseline for issue #455 and the #454
profile-registry / audit-data moat. It decides the physical source of truth for
local registry state before the later package, MCP, sync, inspector, and shared
service work lands.

Decision:

1. Use existing RocksDB `CF_PROFILES` for first-class local registry rows.
2. Use `CF_KV` only for tiny registry head/pointer rows.
3. Do not add a new registry-specific column family for the M5 v0 registry
   model; schema pressure can be revisited after runtime tools prove the access
   pattern needs it.
4. Keep authored profile TOML as authored source. Installed registry state is
   a separate inspectable JSON row set that links profile TOML/package metadata,
   provenance, trust, compatibility, quality, and audit evidence.

Runtime tools now write these rows through Synapse: `profile_registry_install`
validates a local package manifest/profile TOML, verifies signed trust when
policy requires it, and writes source, package, profile, installed,
compatibility, trust-root, optional quality-link, and source-head rows;
failed trust verification writes a quarantine row instead of install rows.
`profile_registry_disable` updates installed state; `profile_registry_rollback`
rewrites the installed row to a prior trusted/local-validated package and writes
a rollback audit row; `profile_registry_export` and `profile_registry_import`
move local registry bundles and offline contribution bundles. Contribution
bundles carry deterministic component hashes, redacted action-audit evidence,
and quality-summary rows; import stages their evidence under
`profile_registry/v1/contribution/` instead of copying redacted shared evidence
into `CF_ACTION_LOG`. Contribution export strips path-like registry metadata
fields before writing the shared bundle file. Contribution import also runs a
local risk review before active-row writes: hostile or fake-quality bundles
write only a quarantined contribution row, while accepted contribution rows are
still de-ranked until local success evidence exists on this host. Manual FSV
must verify them by reading `CF_PROFILES` / `CF_KV`, local bundle files, and
registry-specific readback tools. The fixtures in
`docs/computergames/fixtures/profile_registry_data_model/` are synthetic row
SoTs for this docs/data-model baseline.

## 2. Column-family use

| Column family | Registry role | Why |
|---|---|---|
| `CF_PROFILES` | Primary profile-registry row store | Already reserved for cached profile/registry rows and quality snapshots; inspectable with existing storage readback. |
| `CF_KV` | Small registry head pointers | Bounded key-value extension for active registry index/head metadata. |
| `CF_ACTION_LOG` | Runtime action evidence | Linked from registry quality/audit pointers; not duplicated into registry rows. |
| `CF_REFLEX_AUDIT` | Runtime reflex evidence | Linked from registry quality/audit pointers; not duplicated into registry rows. |
| `CF_EVENTS` / `CF_OBSERVATIONS` / `CF_SESSIONS` | Session and observation evidence | Linked by row ranges/hashes when needed. |

No new column family is introduced by this decision. The registry can still be
manually verified because `CF_PROFILES` row keys are human-readable UTF-8.

## 3. Key namespaces

All registry keys are UTF-8 and versioned under `profile_registry/v1/`.

| Row kind | CF | Key |
|---|---|---|
| Registry source | `CF_PROFILES` | `profile_registry/v1/source/<source_id>` |
| Profile package | `CF_PROFILES` | `profile_registry/v1/package/<package_id>/<package_version>` |
| Profile version | `CF_PROFILES` | `profile_registry/v1/profile/<profile_id>/<profile_version>` |
| Installed profile | `CF_PROFILES` | `profile_registry/v1/installed/<profile_id>` |
| Compatibility target | `CF_PROFILES` | `profile_registry/v1/compat/<target_id>/<profile_id>/<profile_version>` |
| Curated starter target | `CF_PROFILES` | `profile_registry/v1/curated_target/<seed_set_id>/<target_id>` |
| Quality link | `CF_PROFILES` | `profile_registry/v1/quality_link/<profile_id>/<profile_version>` |
| Trust root | `CF_PROFILES` | `profile_registry/v1/trust_root/<signer_id>/<key_id>` |
| Quarantined package | `CF_PROFILES` | `profile_registry/v1/quarantine/<package_id>/<package_version>/<manifest_digest_prefix>` |
| Rollback event | `CF_PROFILES` | `profile_registry/v1/rollback/<profile_id>/<timestamp>` |
| Contribution bundle | `CF_PROFILES` | `profile_registry/v1/contribution/<profile_id>/<deterministic_bundle_sha256>` |
| Registry head pointer | `CF_KV` | `profile_registry/v1/head/<source_id>` |

`profile_quality/v1/<profile_id>` remains the existing local quality snapshot
key. Registry quality-link rows point to that snapshot instead of copying it.

## 4. Common row envelope

Every registry row value is JSON with this envelope:

```json
{
  "schema_version": 1,
  "row_kind": "profile_package",
  "row_id": "profile.synthetic.governance@0.1.0",
  "created_at": "2026-05-27T02:00:00Z",
  "updated_at": "2026-05-27T02:00:00Z",
  "source_id": "registry.synthetic.local-fixture",
  "state": "active"
}
```

Required envelope fields:

- `schema_version`
- `row_kind`
- `row_id`
- `created_at`
- `updated_at`
- `source_id`
- `state`

Unknown future schema versions fail closed unless a migration issue defines the
upgrade path. Row writes are all-or-nothing at the batch boundary; duplicate or
invalid package rows must not leave partial companion rows.

## 5. Entity model

### 5.1 Registry source

Links a local source id to optional shared/file-backed source metadata.

Required fields beyond the envelope:

- `source_kind`
- `base_url` or `root_path`
- `auth_mode`
- `trust_policy_id`
- `offline_usable`
- `last_health_status`

### 5.2 Profile package

Represents an installable package version.
The manifest bytes are defined by
[`23_profile_package_manifest.md`](23_profile_package_manifest.md); this row
stores the manifest/package digests and registry state after validation.

Required fields beyond the envelope:

- `package_id`
- `package_version`
- `manifest_digest`
- `package_digest`
- `license_spdx`
- `governance_manifest_key`
- `trust_status`
- `signature_status`
- `signer_id`
- `key_id`
- `trust_root_key`
- `signature_payload_digest`
- `trust_policy_id`
- `trust_required`
- `moderation_status`
- `revoked`
- `profile_versions`
- `provenance`

### 5.3 Profile version

Links a package to a runtime profile id/version.

Required fields beyond the envelope:

- `profile_id`
- `profile_version`
- `package_id`
- `package_version`
- `profile_toml_path`
- `profile_toml_digest`
- `use_scope`
- `schema_version_supported`

### 5.4 Installed profile

Tracks local install/activation state without rewriting authored profile TOML.

Required fields beyond the envelope:

- `profile_id`
- `active_profile_version`
- `installed_package_id`
- `installed_package_version`
- `installed_at`
- `activation_state`
- `operator_overrides_path`
- `trust_status`
- `signature_status`
- `trust_root_key`
- `signature_payload_digest`
- `signer_id`
- `key_id`
- `trust_policy_id`
- `trust_required`

### 5.5 Compatibility target

Stores compatibility by target, not just by profile id.

Required fields beyond the envelope:

- `target_id`
- `target_kind`
- `profile_id`
- `profile_version`
- `compatibility_status`
- `source_quality_snapshot_key`
- `evidence_hash`

### 5.6 Trust root

Stores the public verifier identity used for signed packages accepted by local
policy.

Required fields beyond the envelope:

- `signer_id`
- `key_id`
- `algorithm`
- `public_key_sha256`
- `public_key_hex`
- `trust_domain`
- `operator_owned`

### 5.7 Quarantined package

Records packages rejected by signed-trust policy. Quarantine rows are evidence,
not install state: bad packages must not create package/profile/installed/head
rows.

Required fields beyond the envelope:

- `package_id`
- `package_version`
- `profile_id`
- `manifest_digest`
- `trust_policy_id`
- `signature_status`
- `quarantine_reason`
- `error_code`

### 5.8 Rollback event

Records an installed-row rollback.

Required fields beyond the envelope:

- `profile_id`
- `from_package_id`
- `from_package_version`
- `to_package_id`
- `to_package_version`
- `target_package_key`
- `reason`
- `trust_status`
- `signature_status`
- `trust_root_key`
- `signature_payload_digest`
- `signer_id`
- `key_id`
- `trust_policy_id`

### 5.9 Curated starter target

Links a selected seed target from
[`24_curated_starter_registry.md`](24_curated_starter_registry.md) to the
package/profile/compatibility rows that make it real. This row is written by
`profile_registry_install` only when a manifest declares complete `curated.*`
metadata.

Required fields beyond the envelope:

- `seed_set_id`
- `target_id`
- `tier`
- `priority`
- `status`
- `backlog_issue`
- `profile_id`
- `profile_version`
- `package_id`
- `package_version`
- `target_kind`
- `use_scope`
- `compatibility_status`
- `safe_default_backend_policy`
- `input_backends`
- `minimum_manual_fsv`
- `quality_signal`
- `profile_quality_key`
- `package_key`
- `installed_key`
- `compatibility_key`
- `provenance`

If any `curated.*` metadata exists, the install path fails closed unless the
full required set is present and `curated.target_id` matches a manifest
compatibility target.

### 5.10 Quality link

Points registry rows to audit-derived quality snapshots.

Required fields beyond the envelope:

- `profile_id`
- `profile_version`
- `profile_quality_key`
- `source_cf_ranges`
- `quality_score`
- `sample_count`
- `evidence_hash`

### 5.11 Contribution bundle

Stages an offline contribution bundle imported through `profile_registry_import`
with `bundle_kind = "contribution"`. This row is a review/staging record, not a
runtime action-audit row.

Required fields beyond the envelope:

- `bundle_kind`
- `profile_id`
- `deterministic_bundle_sha256`
- `bundle_file_sha256`
- `registry_rows_read`
- `audit_evidence_rows`
- `quality_summary_rows`
- `registry_rows_sha256`
- `audit_evidence_sha256`
- `quality_summary_sha256`
- `merge_rules`
- `review_state`
- `risk_flags`
- `trust_downgrade_reasons`
- `rank_eligible`
- `quality_weight`
- `external_success_evidence_rows`
- `local_success_evidence_rows`
- `external_quality_claims_trusted`
- `external_sharing_allowed`

Contribution rows store redacted evidence summaries such as tool/status/error
counts, foreground process name, backend, and profile schema version. They do
not store raw file paths, window titles, PIDs, HWNDs, screenshots, clipboard
content, audio, or raw unredacted audit rows.

Contribution row states:

| State | Meaning | Search/ranking impact |
|---|---|---|
| `staged` | Hash-valid contribution passed local abuse review. | Hidden from default registry search and `rank_eligible=false` until local success evidence/promoter review exists. |
| `quarantined` | Import review found hostile metadata, unsafe permission markers, fake quality evidence, profile mismatches, or row-volume abuse. | Only the contribution review row is written; package/profile/head rows from the bundle are not written. |

External quality summaries are advisory only. Imported contribution rows record
`quality_weight = 0` and `external_quality_claims_trusted = false`; future
promotion/ranking must prefer locally verified `profile_quality/v1/<profile_id>`
evidence generated from this host's own audit rows.

## 6. Install/register transaction shape

A successful local package registration writes these rows through the real MCP
`profile_registry_install` path:

1. `profile_registry/v1/source/<source_id>` if missing or changed.
2. `profile_registry/v1/package/<package_id>/<package_version>`.
3. One or more `profile_registry/v1/profile/<profile_id>/<profile_version>`.
4. `profile_registry/v1/installed/<profile_id>` when the package is installed.
5. One or more compatibility rows.
6. One curated starter target row when manifest `curated.*` metadata is present.
7. One trust-root row when a package signature verified against a local root.
8. One quality-link row when audit-derived quality already exists.
9. `CF_KV` head pointer for the source/index only after the rows above succeed.

If parsing/schema validation fails, none of the companion rows are written. If
signature/trust verification fails, `profile_registry_install` writes only a
quarantine row and returns `PROFILE_TRUST_VERIFICATION_FAILED`. If a duplicate
package id/version exists with the same digest, the operation is idempotent and
does not rewrite the row. If it exists with a different digest, the operation
fails with a duplicate-conflict result.

## 7. Validation rules

| Case | Outcome |
|---|---|
| Duplicate package id/version with same digest | Idempotent no-op; row hash unchanged. |
| Duplicate package id/version with different digest | Reject with `duplicate_package_version_conflict`; no companion rows change. |
| Corrupt manifest | Reject with `manifest_decode_failed`; no rows written. |
| Incompatible schema version | Reject with `registry_schema_version_unsupported`; no rows written. |
| Missing governance metadata | Reject per `20_profile_registry_governance.md`. |
| Bad signature or unknown signer where signed policy is required | Write quarantine row only; no activation/install rows. |
| Contribution metadata contains prompt/tool-injection text | Write quarantined contribution row only; no active bundle rows or head rows. |
| Contribution quality score claims more samples than exported evidence or a high score with too little evidence | Write quarantined contribution row only with explicit risk flags. |
| Contribution has no local success evidence | Stage/de-rank with `rank_eligible=false` and `quality_weight=0`; do not treat volume as quality. |
| Partial curated starter metadata or curated target without a matching compatibility target | Reject before install; no companion rows change. |
| Rollback target missing, current, revoked, quarantined, or untrusted | Reject with `PROFILE_ROLLBACK_UNAVAILABLE`; installed row unchanged. |
| Revoked package | Reject install/activation; tombstone can still be written. |

## 8. Manual FSV contract

Runtime FSV for this model must:

1. Read `CF_PROFILES` and `CF_KV` before the trigger and show whether the
   synthetic package key exists.
2. Trigger the real package registration path.
3. Separately read `CF_PROFILES`/`CF_KV` after the trigger and print the exact
   source/package/profile/installed/compatibility/quality-link rows.
4. Exercise duplicate id/version, corrupt manifest, incompatible schema
   version, bad signature, unknown signer, and rollback-target edges and prove
   no silent partial install/activation.

For this docs baseline, the synthetic fixture row files remain the physical SoT
for expected row shape. Runtime acceptance must additionally prove the same row
classes exist in RocksDB after the MCP trigger.

## 9. Fixture row index

| Fixture | Purpose |
|---|---|
| `cf_profiles_source_row.json` | Registry source row in `CF_PROFILES`. |
| `cf_profiles_package_row.json` | Package row with provenance, trust, compatibility, and governance pointer. |
| `cf_profiles_profile_version_row.json` | Runtime profile version row. |
| `cf_profiles_installed_row.json` | Local installed profile row. |
| `cf_profiles_compatibility_row.json` | Compatibility target row. |
| `../curated_starter_registry/cf_profiles_curated_luanti_row.json` | Curated starter target row for `starter.v1` / `luanti.minetest`. |
| `cf_profiles_quality_link_row.json` | Link from registry package to `profile_quality/v1/<profile_id>`. |
| `cf_profiles_trust_root_row.json` | Trust root row for the synthetic Ed25519 signer. |
| `cf_profiles_quarantine_row.json` | Quarantine-only row for a rejected signed package. |
| `cf_profiles_rollback_row.json` | Rollback event row for restoring a prior trusted package. |
| `cf_kv_registry_head_row.json` | Source/index head pointer in `CF_KV`. |
| `edge_duplicate_id_version.toml` | Duplicate id/version conflict edge. |
| `edge_corrupt_manifest.toml` | Corrupt manifest edge. |
| `edge_incompatible_schema_version.toml` | Unsupported schema edge. |
