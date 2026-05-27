# 20 - Profile Registry Governance

## 1. Status and authority

This document is the product governance baseline for issue #470 and the
profile-registry / audit-data moat in #454. It is not legal advice. It defines
the metadata Synapse requires before profile packages, redacted audit bundles,
derived profiles, generated patches, registry promotion, or optional shared
contributions can ship.

The governing product rule is local-first and consent-gated:

1. Installing or using a local profile is not the same action as sharing it.
2. Audit rows are local evidence by default.
3. Export or contribution requires explicit operator consent, redaction,
   provenance, attribution, and a license expression.
4. Missing or invalid governance metadata fails closed: quarantine, refuse
   promotion/export, and preserve enough local audit state to explain why.

Synapse uses SPDX license expressions for machine-readable license metadata.
Repository code and bundled profile fixtures inherit the workspace license
`MIT OR Apache-2.0` unless a profile package manifest explicitly states a more
specific approved expression. Redacted audit contribution bundles must also
carry an explicit SPDX expression before they can leave the local machine.

## 2. Physical sources of truth

| Surface | Source of truth | Required readback |
|---|---|---|
| Profile package | `package_manifest.toml` inside the package, plus future registry index row | `package_id`, `profile_id`, `version`, `license_spdx`, attribution, provenance, revocation state |
| Redacted audit bundle | Bundle manifest plus consent record | `bundle_id`, source profile/package, redaction policy, `operator_consent_id`, `license_spdx`, data classes, raw-data flags |
| Derived/generated profile | Derived package manifest | Parent package/profile/version, source audit bundle ids, generator id, contributor attribution |
| Revocation | Local tombstone / registry revocation record | Revocation id, target package/version, reason, effective time, local behavior |
| Runtime evidence | RocksDB `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`, `CF_EVENTS`, `CF_OBSERVATIONS`, `CF_SESSIONS`, `CF_PROFILES` | Rows linking profile id/version/package to action/reflex/session outcomes |
| Operator-visible readback | MCP `profile_list`, `profile_quality_refresh`, `storage_inspect`, and future registry/audit tools | Loaded metadata plus persisted storage rows, read separately after trigger |

The fixtures in
`docs/computergames/fixtures/profile_registry_governance/` are synthetic SoTs
for the current docs-only governance baseline. Future runtime implementation
issues (#456, #460, #464, #468, and related children) must replace fixture-only
readback with the real package/import/export/registry tool path plus separate
physical source-of-truth inspection.

## 3. Profile package manifest

Every installable profile package must include a manifest with these fields:
The canonical v1 package schema and parser behavior are defined in
[`23_profile_package_manifest.md`](23_profile_package_manifest.md); this
section defines the governance fields that schema must preserve.

| Field | Rule |
|---|---|
| `schema_version` | Integer. Unknown future major versions fail closed. |
| `kind` | `profile_package`. Other kinds are rejected by the profile importer. |
| `package_id` | Stable reverse-DNS or registry-scoped id. |
| `package_version` | Semver. The registry can keep multiple versions. |
| `profile_id` | Runtime profile id loaded by `profile_list`. |
| `profile_version` | Semver copied from the profile TOML. |
| `license_spdx` | Required valid SPDX expression. Empty or missing means no install, promotion, or export. |
| `contribution_terms` | Required for shared contribution paths. Initial policy uses `DCO-1.1` or an explicitly approved replacement. |
| `contributors` / `attribution` | Human-readable attribution and optional stable contributor ids. |
| `provenance` | Source URI, source commit/hash when available, package builder, generated-by metadata, parent packages for derived profiles. |
| `compatibility` | Target app/game, supported-use scope, OS, Synapse schema/tool versions, and benchmark ids. |
| `revocation` | Current revoked flag plus revocation record id if quarantined. |

Apache-2.0 Section 5 covers intentionally submitted contributions to an
Apache-licensed work unless the contributor explicitly states otherwise, but
Synapse still requires manifest metadata so package and registry tooling can
fail closed without interpreting prose. DCO sign-off is an attestation surface,
not a replacement for license metadata.

## 4. Audit contribution bundle manifest

Audit data is useful only if the receiver knows what it is allowed to do with
it and how it was redacted. A redacted audit contribution bundle must include:

| Field | Rule |
|---|---|
| `kind` | `audit_contribution_bundle`. |
| `bundle_id` | Stable id for this export bundle. |
| `license_spdx` | Required for any shared bundle. |
| `operator_consent_id` | Required non-empty id pointing to a local consent record. |
| `redaction_policy_id` | Required policy id and version. |
| `raw_data_included` | Must be `false` for v1 shared contribution bundles. |
| `data_classes` | Explicit list such as `profile_quality_summary`, `action_outcome_counts`, or `compatibility_flags`. |
| `source_profile_package_id` | Package that produced the evidence. |
| `source_quality_snapshot_key` | Example: `profile_quality/v1/<profile_id>` in `CF_PROFILES`. |
| `source_cf_ranges` | Bounded row ranges or hashes used to derive the aggregate. |
| `attribution` | Contributor/operator attribution and source package notices. |

No raw screenshots, raw UIA trees, raw audio, clipboard contents, free-form
window titles, local file paths, account ids, secrets, or unredacted text leave
the local machine in v1 contribution bundles. If a future feature needs a
larger data class, it needs a new issue and manual SoT/FSV acceptance.

The current local runtime path is `profile_registry_export` with
`bundle_kind = "contribution"` followed by `profile_registry_import` into a
separate local registry DB/path. The exported JSON carries deterministic hashes
for registry rows, redacted audit evidence, quality summaries, and the combined
content. Import recomputes those hashes, skips byte-identical duplicate rows,
skips same-deterministic-content contribution rows even when the exact
bundle-file hash differs, fails closed on same-key/different-value conflicts,
and stages contribution evidence under
`CF_PROFILES/profile_registry/v1/contribution/<profile_id>/...`. It does not
import redacted shared evidence into `CF_ACTION_LOG`. Contribution export also
strips path-like registry metadata fields from shared bundle rows.

Contribution import is a local review gate, not a trust shortcut. Before active
registry/head writes, Synapse reviews the bundle for prompt/tool-injection text,
unsafe permission escalation markers, profile-id mismatches, fake quality
claims, excessive row/evidence volume, and quality scores not supported by the
exported redacted evidence. Hostile bundles write only a quarantined
`profile_contribution_bundle` row with explicit `risk_flags` and
`trust_downgrade_reasons`. Hash-valid bundles that pass abuse review are still
`staged`, `rank_eligible = false`, `quality_weight = 0`, and
`external_quality_claims_trusted = false` until this configured host has local
success evidence. This prevents contribution volume or self-claimed scores from
outranking verified local `profile_quality/v1/<profile_id>` rows.

## 5. Attribution and provenance

Attribution is preserved across these flows:

| Flow | Required provenance |
|---|---|
| Forked profile | Original package id/version, source URI, source commit/hash, original attribution notice, modifier attribution |
| Derived profile from audit evidence | Parent profile package, source audit bundle ids or local `CF_PROFILES` snapshot key, generator id, operator id or redacted contributor id |
| Generated patch | Base profile version, prompt/tool/agent id if recorded, audit evidence key range/hash, generated-at timestamp |
| Registry promotion | Local package hash, signing identity, review/moderation outcome, promoted version |

A derived profile without attribution is not an unknown-quality profile; it is
a governance failure. The importer must refuse promotion/export and keep a
diagnostic record that names `missing_attribution`.

## 6. Revocation and deletion behavior

Revocation is append-only. Deletion is local data removal or registry
discoverability removal, not an erasure of historical facts that explain past
actions.

Local installed package:

1. Remove the package from the active registry index.
2. Write a local revocation/tombstone record.
3. Quarantine the package bytes if retained for audit.
4. Refuse new activations, profile-quality promotion, and export from the
   revoked package.
5. Keep prior action/reflex/session audit rows until their normal retention
   expiry, with package/profile id fields intact so past behavior remains
   explainable.

Exported but not shared bundle:

1. Remove it from any local outbound queue.
2. Write a local revocation record for the bundle id.
3. Refuse re-export until a new consent and manifest are created.

Already shared contribution:

1. Publish a future shared-registry revocation tombstone.
2. Clients that read the tombstone quarantine the package and refuse new
   installs/activations.
3. The shared registry can remove discoverability, but Synapse must not claim
   it can delete third-party copies.

Operator personal-data deletion is handled by the local retention/export
surface. A deletion request can wipe local export bundles and eligible local
RocksDB rows, but it must not rewrite history by pretending a package was never
used if retained action/reflex/session audit rows still exist.

## 7. Fail-closed rules

| Condition | Required outcome |
|---|---|
| Missing `license_spdx` | Reject install/export/promotion; quarantine with `missing_license`. |
| Invalid SPDX expression | Reject; quarantine with `invalid_license_expression`. |
| Revoked package update | Reject new activation/install unless the update explicitly resolves the revocation through a replacement package id/version. |
| Derived profile without attribution | Reject export/promotion; keep diagnostic `missing_attribution`. |
| Audit bundle missing consent | Reject export; keep local audit data local. |
| Audit bundle includes raw sensitive classes | Reject export; require a new redaction policy and explicit issue acceptance. |
| Unknown manifest major version | Reject until a migration/import issue defines the new schema. |
| Contribution bundle contains prompt/tool-injection text in metadata | Quarantine contribution row only with `metadata_prompt_injection_text`; no active rows. |
| Contribution bundle claims high quality with too little/mismatched evidence | Quarantine contribution row only with the quality risk reason. |
| Contribution bundle has volume but no local success evidence | Stage/de-rank with `rank_eligible = false` and `quality_weight = 0`. |
| Package asks for remote execution or non-local execution | Reject before install; no package/profile/installed/head rows. |

These are not warnings. They are terminal governance errors until the package or
bundle is corrected.

## 8. Manual FSV contract

For governance work, FSV must use the real runtime surface when one exists and
then read the separate physical SoT. The minimum acceptance pattern is:

1. **Before read:** read the package/bundle/registry/consent SoT and show
   whether license, attribution, provenance, consent, and revocation metadata
   exists.
2. **Trigger:** install/import/export/promote/revoke using the real Synapse MCP
   or daemon path when implemented. For this docs baseline, the trigger is
   creating the synthetic manifests and generated systemspec references.
3. **After read:** read the manifest, bundle, registry index/tombstone, consent
   record, RocksDB rows, and MCP readback separately. Do not rely on the trigger
   return value.
4. **Happy path:** a synthetic profile package and redacted audit bundle with
   known license, attribution, provenance, consent, and non-revoked state.
5. **Edge cases:** at minimum, missing license, revoked package update, derived
   profile without attribution, and audit export without consent.

The current synthetic fixture set covers those cases as physical TOML bytes.
Future importer/exporter issues must run the same known inputs through the
actual package/export tools and read the resulting registry, bundle, consent,
and RocksDB SoTs.

## 9. Reference sources

- SPDX License List: https://spdx.org/licenses/
- SPDX License Expressions: https://spdx.github.io/spdx-spec/v2.2.2/SPDX-license-expressions/
- Apache License 2.0: https://www.apache.org/licenses/LICENSE-2.0.html
- Apache provenance FAQ: https://www.apache.org/foundation/license-faq.html
- Developer Certificate of Origin 1.1: https://developercertificate.org/
- GitHub licensing docs: https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository
- Creative Commons Attribution 4.0: https://creativecommons.org/licenses/by/4.0/
