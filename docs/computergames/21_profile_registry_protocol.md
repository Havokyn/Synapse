# 21 - Optional Shared Registry Protocol

## 1. Status and authority

This document is the architecture baseline for issue #469 and the optional
shared registry boundary in the #454 profile-registry / audit-data moat. The
shared service is optional. Synapse must remain useful with only local profile
TOML files, local registry package files, local audit rows, and local quality
snapshots.

The protocol exists to make the eventual network effect explicit before any
hosted/shared service appears:

1. Local registry use does not require an account, network, or shared service.
2. Search, fetch, update metadata, contribution submission, moderation, signer
   metadata, and revocation are separate state surfaces.
3. Consent/export bundles from #460/#464 are the only audit-data input to a
   shared registry.
4. Moderation is explicit state, not hidden service behavior.
5. Update metadata cannot silently downgrade, roll back, or auto-activate a
   package.

The fixtures in
`docs/computergames/fixtures/profile_registry_protocol/` are synthetic SoTs for
this docs-only boundary. Future runtime issues (#458, #464, #468, and #455/#456
dependencies) must implement real tools or daemon paths and then read the
physical local registry, contribution, moderation, and RocksDB SoTs.

## 2. Design principles

| Principle | Rule |
|---|---|
| Local-first | A missing or unavailable shared source never breaks installed local profiles. |
| Consent-gated | Shared audit contribution requires a local consent/export bundle with governance metadata from `20_profile_registry_governance.md`. |
| Content-addressed | Package bytes and metadata carry digests; fetch results are checked against metadata before use. |
| Explicit trust | Signer/trust state is read as metadata and enforced by local policy. |
| Explicit moderation | Submission status is a record with `queued`, `approved`, `rejected`, `quarantined`, or `revoked`; clients do not infer success from HTTP 200 alone. |
| No auto-activation | Fetch/update can stage packages; activation remains a separate local decision and runtime policy gate. |
| Fail closed | Bad digest, missing consent, moderation rejection, revoked package, stale metadata, or rollback attempt stops install/promotion/export. |

## 3. Physical sources of truth

| Surface | Future local SoT | Synthetic fixture SoT |
|---|---|---|
| Registry source config | `%APPDATA%/synapse/registry_sources.toml` | `registry_source.toml` |
| Local registry index | `%LOCALAPPDATA%/synapse/registry/index.toml` or RocksDB `CF_PROFILES` registry rows | `local_registry_index.toml` |
| Search response | MCP `profile_registry_search` readback plus local cached response | `search_response.json` |
| Package fetch metadata | Package manifest, digest, signer metadata | `package_fetch_manifest.toml` |
| Contribution submission | Local outbound queue/export bundle plus service submission record | `contribution_submission.toml` |
| Moderation status | Moderation queue record keyed by contribution id | `moderation_rejection_record.toml` |
| Update metadata | Registry update metadata and local installed-version row | `edge_stale_rollback_update.toml` |
| Source outage | Registry source health/cache state | `edge_unavailable_registry_source.toml` |

## 4. Protocol boundary

The optional shared service boundary is `Synapse Registry Protocol v0`. It can
be hosted, local-only, or file-backed during development. The MCP/runtime layer
owns local policy; the service provides metadata and blobs.

| Operation | Shape | Required SoT after trigger |
|---|---|---|
| Source health | `GET /v0/health` | source id, protocol version, cache max age, service time |
| Profile search | `GET /v0/profiles/search?target=&profile_id=&use_scope=&schema_version=` | query, result package ids, versions, digests, moderation status, quality summary |
| Package manifest | `GET /v0/packages/{package_id}/versions/{version}/manifest` | package metadata, digest, signer, compatibility matrix, governance metadata |
| Package blob | `GET /v0/packages/{package_id}/blobs/{digest}` | content digest match and staged local package path |
| Update metadata | `GET /v0/packages/{package_id}/updates?installed_version=&channel=` | latest version, monotonic version proof, snapshot timestamp/expiry |
| Contribution submit | `POST /v0/contributions/audit-bundles` | contribution id, source bundle id, consent id, moderation queue id |
| Moderation status | `GET /v0/contributions/{contribution_id}/moderation` | explicit status, reason code, reviewer/system source, timestamps |
| Trust metadata | `GET /v0/trust/signers/{signer_id}` | signer id, key id, transparency reference if any, trust policy state |
| Revocations | `GET /v0/revocations?since=` | package/contribution tombstones and replacement metadata |

Read-only search/fetch MAY be anonymous for a public source. Contribution
submission requires an operator-approved account/token only when the service
actually ingests shared data. Local development can use a file-backed source
with `auth_mode = "none"`.

## 5. Data objects

### 5.1 RegistrySource

Required fields:

- `source_id`
- `kind` (`hosted_http`, `local_file`, or `fixture`)
- `base_url` or `root_path`
- `enabled`
- `auth_mode`
- `trust_policy_id`
- `cache_policy`
- `last_health_status`
- `last_successful_sync_at`

### 5.2 SearchResult

Required fields:

- `package_id`
- `package_version`
- `profile_id`
- `profile_version`
- `manifest_digest`
- `package_digest`
- `moderation_status`
- `trust_status`
- `quality_summary`
- `compatibility`

### 5.3 ContributionSubmission

Required fields:

- `contribution_id`
- `source_bundle_id`
- `operator_consent_id`
- `redaction_policy_id`
- `license_spdx`
- `submission_status`
- `moderation_queue_id`
- `service_receipt_digest`

### 5.4 ModerationRecord

Required fields:

- `moderation_queue_id`
- `contribution_id`
- `status`
- `reason_code`
- `reason`
- `reviewed_at`
- `effective_package_state`

## 6. Auth and account boundary

Local profile registry use never requires credentials. A shared source may
support anonymous read-only search/fetch, but write/contribution operations
require explicit operator configuration. Tokens, if any, live in the configured
host's secret/config surface and are never stored in profile packages or audit
bundles.

No GitHub, cloud, or Synapse account is required for local development. A
file-backed fixture source with `auth_mode = "none"` is the default dev path.
If a future hosted service requires credentials, that issue must define the
token storage SoT, consent prompt, revocation path, and manual readback.

## 7. Update and rollback boundary

Update metadata is advisory until the operator or policy explicitly stages and
activates a package. A valid update path must prove:

1. The target package id matches the installed package id.
2. The offered version is newer than the installed version for the selected
   channel.
3. Snapshot metadata is fresh under the source cache policy.
4. The manifest digest and package digest match fetched bytes.
5. The package is not revoked and moderation status is acceptable.

A lower version, expired snapshot, missing snapshot timestamp, digest mismatch,
or revoked target is a rollback/stale metadata edge and must be rejected.

## 8. Moderation and abuse-review queues

Moderation state is part of the protocol:

| Status | Meaning | Client behavior |
|---|---|---|
| `queued` | Service accepted receipt, review not complete | Do not publish as searchable approved profile |
| `needs_review` | Automated checks need human or stricter review | Keep local contribution queued; no public use |
| `approved` | Contribution can affect search/quality summary | Eligible for source index after trust policy passes |
| `rejected` | Contribution failed policy | Record reason; do not retry unchanged |
| `quarantined` | Later evidence made package/contribution unsafe | Refuse install/activation/export; keep tombstone |
| `revoked` | Contributor/operator/service revoked it | Refuse new use; preserve explainability tombstone |

Moderation rejects include a structured `reason_code`, for example
`missing_consent`, `invalid_license`, `poisoning_signal`, `low_quality_sample`,
`unsupported_target`, `raw_sensitive_data`, or `rollback_detected`.

## 9. Offline contribution flow

The shared-registry boundary consumes only local artifacts already produced by
other workstreams:

1. #457 writes profile-linked runtime evidence.
2. #461 scores local quality into `CF_PROFILES`.
3. #460 creates operator consent and redaction policy state.
4. #464 packages an offline contribution bundle.
5. This protocol submits that bundle and reads a contribution/moderation record.

If the source is unavailable, the outbound queue remains local and the installed
registry remains usable. No contribution is lost or silently marked accepted.

## 10. Manual FSV contract

For runtime work, FSV must trigger real registry tools and then separately read
local registry files/RocksDB rows plus registry-source metadata. For this docs
baseline, the trigger is creation of the synthetic protocol fixtures.

Minimum cases:

1. Happy path: configured fixture source -> search result -> package manifest
   -> contribution submission queued.
2. Unavailable source: source health shows unavailable; local cache/index stays
   usable.
3. Moderation rejection: contribution status reads `rejected` with a reason
   code and no approved search result is implied.
4. Stale/rollback update: update metadata offers a lower/expired version and
   local policy reads `outcome = "reject"`.

## 11. References

- OCI Distribution Specification: https://github.com/opencontainers/distribution-spec/blob/main/spec.md
- The Update Framework: https://theupdateframework.io/
- Sigstore docs: https://docs.sigstore.dev/
- SLSA: https://slsa.dev/
- Synapse governance baseline: `20_profile_registry_governance.md`
