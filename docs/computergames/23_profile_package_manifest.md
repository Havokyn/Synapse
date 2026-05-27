# 23 - Profile Package Manifest

## 1. Status and authority

This document is the package manifest baseline for issue #456 and the #454
profile-registry / audit-data moat. It makes profile packages portable without
losing provenance, compatibility, safety scope, version history, or hash
evidence.

Runtime profile TOML remains the authored profile source. A package manifest
is the transport and registry metadata source that says where the profile came
from, where it is safe to use, what it depends on, and which bytes were
validated before install.

Implemented local API:

- `synapse_profiles::parse_package_manifest_file`
- `synapse_profiles::parse_package_manifest_bytes`
- `synapse_profiles::parse_package_manifest_bytes_with_digest`
- `synapse_profiles::package_manifest_digest`
- `synapse_profiles::package_signature_payload`
- `synapse_profiles::package_signature_payload_digest`

The parser is fail-closed: unknown TOML fields, unsupported schema versions,
missing provenance, ambiguous targets, incompatible local assumptions, unsafe
permissions, invalid semantic versions, malformed SHA-256 digests, malformed
trust policies, and malformed package signatures are errors.

## 2. Physical sources of truth

| Surface | Source of truth | Required readback |
|---|---|---|
| Package manifest | `package_manifest.toml` in the package or local package dir | id/version, profile id/version, provenance, targets, assumptions, permissions, hashes |
| Package digest | Registry/fetch metadata or local package row | expected manifest digest compared with actual manifest bytes |
| Signature payload | Deterministic payload from `package_signature_payload(manifest)` | payload digest and Ed25519 signature verification result |
| Trust root | `CF_PROFILES` key `profile_registry/v1/trust_root/<signer_id>/<key_id_hex>` | signer id, key id, algorithm, public key, trust source |
| Installed package registry row | `CF_PROFILES` key `profile_registry/v1/package/<package_id>/<package_version>` | manifest path/digest, package digest, profile id/version, target ids, trust/signature/moderation state |
| Quarantine row | `CF_PROFILES` key `profile_registry/v1/quarantine/<package_id>/<package_version>/<digest-prefix>` | failed trust status, signature status, reason, manifest digest, no activation |
| Rollback row | `CF_PROFILES` key `profile_registry/v1/rollback/<profile_id>/<timestamp>` | previous package/version, restored package/version, reason, trust status |
| Runtime profile | `crates/synapse-profiles/profiles/*.toml` or installed profile dir | authored profile bytes and profile parser readback |
| Runtime install/rollback tools | MCP `profile_registry_install` / `profile_registry_rollback` | tool trigger plus separate manifest/`CF_PROFILES`/`CF_KV` readback |

The current fixture SoTs live in
`docs/computergames/fixtures/profile_package_manifest/`. Future runtime issues
#458 and #468 must route the same known manifests through a real Synapse tool
and then read the installed files plus `CF_PROFILES` rows directly.

## 3. Manifest shape

Top-level required fields:

| Field | Rule |
|---|---|
| `schema_version` | Current value `1`; higher values fail closed. |
| `kind` | Must be `profile_package`. |
| `package_id` | Lowercase registry-scoped id with a namespace separator, for example `profile.luanti.minetest`. |
| `package_version` | Semantic Versioning 2.0.0 `major.minor.patch` form. |
| `profile_id` | Runtime profile id, for example `luanti.minetest`. |
| `profile_version` | Semantic Versioning 2.0.0 profile version. |
| `created_at` | RFC3339 timestamp. |
| `author` | Name, contact, attribution notice. |
| `source` | Source kind, URI, revision, builder id, generator id. |
| `targets` | One or more compatibility targets. |
| `assumptions` | OS, Synapse schema version, display/DPI assumptions, benchmark ids. |
| `input` | Supported action backends plus firmware/model dependencies. |
| `permissions` | SPDX license expression, contribution terms, use scope, execution flags, contribution/export flags. |
| `changelog` | One or more versioned changelog entries. |
| `hashes` | Profile TOML digest, package digest, optional asset digests. |
| `files` | Profile TOML path and optional asset paths inside the package. |
| `trust` | Optional package trust policy and required signer ids. |
| `signatures` | Optional detached package signatures over the deterministic signature payload. |

Each compatibility target must identify the target and include at least one
match surface: `app_id`, `process_name`, `title_regex`, or `steam_appid`.
Regexes must compile locally before the package can be staged.

Trust fields:

```toml
[trust]
policy = "signed_required"          # or "local_unsigned_allowed"
required_signers = ["synapse.fixture.signer"]

[[signatures]]
signer_id = "synapse.fixture.signer"
key_id = "sha256:<64 lowercase hex>"
algorithm = "ed25519"
signature = "ed25519:<128 lowercase hex>"
```

`trust.policy = "signed_required"` requires at least one signature. The
runtime install tool may also force this with `trust_policy = "signed_required"`
even when a local manifest permits unsigned local staging. Signature
verification is over `package_signature_payload(manifest)`, which excludes the
`signatures` table so signatures do not sign themselves.

## 4. Profile source mapping

All profile sources map into the same package shape:

| Source | Manifest `source.kind` | Required provenance |
|---|---|---|
| Bundled profile | `bundled` | repository URI, revision, builder, generator, bundled profile path |
| Local user profile | `local_user` | `file://` URI or configured profile dir, local revision/id, builder/generator |
| Registry/community profile | `registry` | registry source URI, package revision/digest, signer/trust metadata in the registry row |
| Synthetic fixture | `synthetic_fixture` | fixture path and issue/workstream provenance |

This keeps local-only profiles, bundled profiles, and future shared-registry
profiles inspectable with one parser and one registry row model.

## 5. Fail-closed validation

The `synapse-profiles` parser rejects these cases:

| Case | Required outcome |
|---|---|
| Missing `source` / provenance | Reject before install; no registry row written. |
| Unknown future `schema_version` | Reject with profile version incompatibility. |
| `kind` not `profile_package` | Reject. |
| Invalid `package_version` / `profile_version` | Reject. |
| Unsupported OS or mismatched Synapse schema version | Reject as incompatible target metadata. |
| No compatibility target or target with no match surface | Reject as ambiguous. |
| Invalid target `title_regex` | Reject. |
| Empty backend list or duplicate backend | Reject. |
| `use_scope = "unknown"` | Reject for installable packages. |
| `permissions.execution.local_only != true` or `remote_server_allowed != false` | Reject; v1 installable packages are local-only. |
| `permissions.contribution.share_audit_allowed = true` without `export_allowed = true` | Reject. |
| Metadata/changelog/provenance text contains prompt/tool-injection markers | Reject before install; metadata is data, not agent instructions. |
| Missing or unapproved package license | Reject. |
| Malformed SHA-256 digest | Reject. |
| Expected manifest digest differs from actual manifest bytes | Reject before metadata is trusted. |
| Unknown `trust.policy` | Reject before install. |
| `signed_required` with no signature | Reject before activation and write only a quarantine row in the runtime install path. |
| Unknown signer or untrusted key id | Reject before activation and write only a quarantine row in the runtime install path. |
| Invalid Ed25519 signature | Reject before activation and write only a quarantine row in the runtime install path. |

The current approved profile-package license expressions are `MIT`,
`Apache-2.0`, and `MIT OR Apache-2.0`. Wider licensing can be added only by a
separate governance issue that updates #470's rules.

## 6. Installed registry row

After a future install trigger succeeds, the package manifest must be
represented by the data-model row from `22_profile_registry_data_model.md`.
The minimum row must include:

- `CF_PROFILES`
- key `profile_registry/v1/package/<package_id>/<package_version>`
- `manifest_path`
- `manifest_digest`
- `package_digest`
- `profile_id`
- `profile_version`
- target ids
- `license_spdx`
- `trust_status`
- `signature_status`
- `trust_root_key`
- `signature_payload_digest`
- trust/moderation/revocation state

The fixture `installed_registry_row.json` shows the row for
`profile.luanti.minetest@0.1.0` and points to
`happy_package_manifest.toml` with the manifest byte digest recorded in the row.

Successful signed installs also write the trust root row that was used for
verification. Failed signed installs do not write package/profile/installed/head
rows; they write only a quarantine row with `activation_state = "quarantined"`
and the trust failure reason. `profile_registry_rollback` restores an installed
row only to a prior package row whose `trust_status` is `trusted` or
`local_validated`, and the restored installed row must carry the target
package's trust root, signer/key, trust policy, and signature payload digest.

When a manifest includes the curated starter metadata from
[`24_curated_starter_registry.md`](24_curated_starter_registry.md), successful
install also writes
`profile_registry/v1/curated_target/<seed_set_id>/<target_id>`. Partial
`curated.*` metadata or a curated target id that is absent from the manifest
compatibility targets fails closed before install rows are written.

## 7. Manual FSV contract

Runtime FSV for package install must:

1. Read the profile package directory and `CF_PROFILES` before the trigger and
   show the synthetic package row is absent.
2. Trigger the real install/validate path with a synthetic package whose fields
   and hashes are known.
3. Read the installed `package_manifest.toml` and `CF_PROFILES` package row
   separately.
4. Prove the readback fields match the expected id, version, provenance,
   target, assumptions, permissions, package digest, manifest digest, trust
   status, signature status, trust-root key, and signature payload digest.
5. Exercise missing provenance, incompatible target metadata, manifest hash
   mismatch, bad signature, unknown signer, and rollback-from-bad-update
   cases; for each, read before/after SoT and prove rejected packages do not
   activate.

For this issue, the parser/validator and physical fixtures are the local SoT.
Downstream registry tool issues must not use those fixtures as a substitute for
real runtime install FSV.

## 8. Fixture index

| Fixture | Purpose |
|---|---|
| `happy_package_manifest.toml` | Valid Luanti benchmark package manifest with provenance, compatibility, permissions, dependencies, and hashes. |
| `installed_registry_row.json` | `CF_PROFILES` row that a successful install must produce for the happy package. |
| `edge_missing_provenance_manifest.toml` | Invalid package missing the required `source` provenance table. |
| `edge_incompatible_target_manifest.toml` | Invalid package with unsupported OS target metadata. |
| `edge_manifest_hash_mismatch.toml` | Expected policy for digest mismatch before metadata trust/install. |
| `signed_good_package_manifest.toml` | Valid signed Luanti package; synthetic fixture signer; expected payload digest `sha256:a39fc832f873ed6ae62ee962f52b6bed705c8683beda44f65384dca85409df3e`. |
| `signed_update_package_manifest.toml` | Valid signed update used to prove rollback to a prior trusted package; expected payload digest `sha256:93eb5e7fa64999919ecdb59cf09ad2aec9b8c430fc493cdd8be466a2f8757145`. |
| `edge_bad_signature_package_manifest.toml` | Signed-required package with a known signer and intentionally corrupted signature; expected quarantine-only install failure. |
| `edge_unknown_signer_package_manifest.toml` | Signed-required package whose signer is absent from the local trust roots; expected quarantine-only install failure. |
| `../curated_starter_registry/curated_luanti_package_manifest.toml` | Valid curated seed package that writes a `curated_profile_target` row for `luanti.minetest`. |
| `../curated_starter_registry/edge_unknown_use_scope_manifest.toml` | Invalid curated package with `use_scope = "unknown"`. |
| `../curated_starter_registry/edge_missing_compatibility_manifest.toml` | Invalid curated package with no compatibility target. |

## 9. References

- Semantic Versioning 2.0.0: https://semver.org/
- SPDX License Expressions: https://spdx.github.io/spdx-spec/v2.2.2/SPDX-license-expressions/
- OCI Image Manifest Specification: https://specs.opencontainers.org/image-spec/manifest/
- SLSA Provenance: https://slsa.dev/spec/v1.2/
- The Update Framework Specification: https://theupdateframework.github.io/specification/v1.0.26/
- Sigstore signing and verification docs: https://docs.sigstore.dev/cosign/verifying/verify/
