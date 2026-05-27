# 06 — M5: Production Polish (3-4 weeks)

> Read after `05_m4_hardware_hid_first_game.md` is closed (`v0.1.0-m4`
> tagged). Gets a full self-contained M2-style rewrite as the first M5 task.
> **All global invariants apply** (no backcompat, no mocks gate completion,
> FSV with source-of-truth read-back, Natural-only motion, manual
> configured-host shipping gate; local checks are regression support only).
> The full v1.0 release gate remains blocked by M4, but the P1
> profile-registry / audit-data moat track is active now via #454/#455-#470.

PRD: `docs/computergames/15_roadmap_and_milestones.md` §7. Build/install: `14_build_and_packaging.md`. Acceptance: `15 §10`. Manual test plan: `13 §15`. Doctrine: `00_methodology.md` + `07_cross_cutting.md`.

## Mission (Occam's razor)

**Sign an installer, fill out the debug overlay (currently a 3-LoC main.rs in `synapse-overlay`), bundle 10+ profiles including the M4 `minecraft.java` lighthouse, make the profile-registry / audit-data moat real, ship the VLM `describe` tool, and prove an 8 h soak.** Every other M5 clause is a consequence of that sentence plus the global invariants.

## Goal

v1.0 ship-ready: signed installer, 10+ bundled profiles (4 from M3 + 1 from M4 + 5+ new), debug overlay (fills out `synapse-overlay` crate from M0 stub), VLM `describe` (Florence-2-base, downloaded on first call — never bundled), Grafana dashboards, soak 8 h clean, setup wizard, tray icon, public release on GitHub Releases + crates.io + winget submission.

The P1 strategic data loop from #454 is part of "ship-ready": profile used ->
audit evidence captured -> quality/compatibility learned -> profile improved
-> registry distributes a better profile -> more evidence. Child issues
#455-#470 are the current ledger for data model, packages, audit linkage, MCP
tools, trust policy, consent/export, scoring, authoring, retention, sync,
abuse defenses, seed registry, docs, inspector, shared-registry boundary, and
governance.

Manual FSV for registry/audit work must trigger the real Synapse runtime path
where one exists and then read the physical source of truth: profile TOML,
future registry index/package files, RocksDB `CF_ACTION_LOG`,
`CF_REFLEX_AUDIT`, `CF_EVENTS`, `CF_OBSERVATIONS`, `CF_SESSIONS`, and
`CF_PROFILES`, consent/export bundles, and MCP readbacks. CI, GitHub Actions,
tests, scripts, and benchmarks are supporting evidence only.

Contribution rights, attribution, provenance, license metadata, revocation, and
deletion semantics are governed by
`docs/computergames/20_profile_registry_governance.md`; no profile package or
redacted audit bundle can be shared or promoted without those physical manifest
fields.

Optional shared-registry protocol and moderation boundaries are governed by
`docs/computergames/21_profile_registry_protocol.md`. The shared service is not
required for local use; read-only source config, package fetch, update
metadata, contribution submission, moderation records, trust metadata, and
revocations remain distinct source-of-truth surfaces.

Local registry storage/data-model boundaries are governed by
`docs/computergames/22_profile_registry_data_model.md`. Registry source,
package, profile-version, installed-profile, compatibility, and quality-link
rows live under namespaced `CF_PROFILES` keys; `CF_KV` holds only tiny head
pointers.

Profile package manifest boundaries are governed by
`docs/computergames/23_profile_package_manifest.md`. Package manifests carry
id/version, provenance, compatibility targets, local assumptions,
permissions/use scope, changelog, profile/package hashes, and file mappings.
The `synapse-profiles` parser rejects missing provenance, incompatible target
metadata, unsafe permissions, invalid semver, and manifest digest mismatches
before a future registry install path can stage rows.

## Demo gate

Fresh Windows 11 VM (no Synapse pre-installed) → operator runs `synapse-mcp setup` wizard → Claude Desktop completes the multi-app scenario per `15 §7`:

1. Open VS Code + write a small Rust file
2. Run `cargo build` in Windows Terminal
3. Switch to Chrome, search, read a result
4. Switch to Minecraft, play 2 min
5. Switch to a music player, control playback

Total token cost ≤ 30 K across the whole sequence.

---

## Inputs

- M4 demo gate passed; `v0.1.0-m4` tag cut
- Clean Windows 11 VM (or Hyper-V Sandbox) for fresh-install testing
- Code-signing cert (self-signed at v1.0; community/EV cert tracked as separate workstream)
- `wix-installer` (WiX Toolset v4+) available
- Reference machine for perf gates: RTX 3060 + 8-core CPU
- Reference machine for soak: dedicated runner, 16 GB RAM, 5 GB free disk
- M5 starting surface (verified at M4 close): 33 MCP tools live (30 shipped at M3 — 6 M1 + 9 M2 + 11 M3 reflex/profile/replay/audio + 4 M3 `storage_*` diagnostics — plus the 3 M4 tools `act_combo`/`act_run_shell`/`act_launch`). The active M5 registry/audit moat work adds `profile_quality_refresh`, six `profile_authoring_*` candidate tools, seven `profile_registry_*` tools including rollback, `audit_intelligence_query`, `audit_export_consent_set`, and `audit_export_bundle`, bringing the current live surface to 50 before the later VLM `describe` work.
- M5 starting profile bundle: 5 (`notepad`, `vscode`, `chrome`, `terminal` from M3 + `minecraft.java` from M4)
- `synapse-overlay` is still the 3-LoC binary skeleton from M0; first M5 task fills it out

---

## Deliverables

### Installer + distribution (`14 §6`)

| Artifact | Tooling | Notes |
|---|---|---|
| `SynapseSetup-x.y.z.msi` | WiX | bundled models (optional checkboxes) + profiles + `pico-hid.uf2` + ViGEmBus install option (calls Nefarius signed installer) |
| `synapse-mcp.exe`, `synapse-overlay.exe` | `cargo build --release` + `signtool` w/ SHA256 + timestamp authority | per `14 §11` |
| `synapse-portable-x.y.z-windows-x64.zip` | manual zip | air-gapped install |
| `synapse-pico-hid-x.y.z.uf2` | `firmware/pico-hid` build pipeline | release asset |
| winget manifest PR | manual | post v1.0.0 |
| crates.io publish | `cargo publish -p synapse-core ...` per library crate | binary crate `synapse-mcp` also installable via `cargo install --git ...` |

### Setup wizard (`14 §7`)

`synapse-mcp setup` (interactive; `--non-interactive --accept-defaults` supported):

1. Write-permission check on `%LOCALAPPDATA%\synapse\`
2. License agreement acknowledgment (`08 §7`) → `%APPDATA%\synapse\agreement.json`
3. ViGEmBus detect + optional install
4. Model selection (YOLOv10n alternates / Whisper-tiny / optional VLM)
5. Profile selection (bundled set)
6. Bearer token gen → `%APPDATA%\synapse\token.txt` (Windows ACL: SYSTEM + current user)
7. Optional RP2040 detect + flash
8. Write `%APPDATA%\synapse\config.toml` per `14 §8`
9. Launch `synapse-mcp --mode stdio` + agent-client config instructions

### Debug overlay (`12 §6`)

Crate `synapse-overlay` (filled out from M0 stub):

- `egui` + `eframe` always-on-top transparent window
- Real-time: capture fps, detection p99, action queue depth, active reflex list + fire counts, recent events tail, disk pressure level, DB size
- Hotkeys: `Ctrl+Alt+Shift+L` toggle, `Ctrl+Alt+Shift+P` panic (shared w/ daemon)
- Read-only — observes telemetry, emits no actions

### Tray icon (`11 §6.2`)

Synapse-mcp adds system tray (`--no-tray` opt-out):

- Status: active / paused / error icons (`assets/tray-icon-*.ico`)
- Menu: Pause / Resume / Disable Reflexes / Open Logs / Quit
- Hover: MCP session count + active profile

### VLM `describe` (`05 §3.3` + `OQ-008`)

- Florence-2-base ONNX (~480 MB; ~25 ms on 5090, 120 ms on 3060)
- **Not bundled in installer.** Downloads on first call w/ explicit consent prompt
- Returns `MODEL_NOT_LOADED` until present
- Add MCP tool `describe` per `05 §3.3`

### Additional bundled profiles (5+)

All new profiles set `mouse_curve_default = "natural"` + `keyboard_dynamics_default = "natural"` per OQ-004 DECIDED. Profile-validation smoke test asserts every bundled `.toml` carries Natural defaults (no `Instant`/`Burst` defaults shipped).

| Profile | Use scope | Notes |
|---|---|---|
| `factorio.toml` | single_player | mod-friendly automation profile |
| `discord.toml` | n/a (productivity) | a11y_only |
| `slack.toml` | n/a | a11y_only |
| `file_explorer.toml` | n/a | a11y_only |
| `<one_fps>.toml` | single_player | TBD free single-player FPS for the M5 demo |
| `roblox_studio.toml` | operator_owned_test | Studio only; runtime experiences start as unknown until profiled |
| Pre-existing M3 (`notepad`, `vscode`, `chrome`, `terminal` — Natural defaults verified) + M4 `minecraft.java` | | already shipped |

Plus inert `unknown` profile templates for local experiments: parseable, no keymap, no bundled game-specific assets, and explicit comments requiring a documented environment before actions are enabled.

### Grafana dashboards (`12 §11`)

`dashboards/*.json`:

- `synapse_overview.json`
- `synapse_perception.json`
- `synapse_action.json`
- `synapse_storage.json`
- `synapse_reflex.json`

### Docs

- `USER_GUIDE.md` (distinct from PRD; operator-facing quick start, troubleshooting, profile authoring)
- `CHANGELOG.md` v1.0.0 entry
- `THIRD-PARTY-LICENSES.md` generated by `cargo about`

### Schema lock

`SCHEMA_VERSION = "1"`. Post-v1 schema changes require ADR + migration (or DB wipe w/ release-note warning per `OQ-028`).

### Error codes (round out catalog)

```
SAFETY_KILLSWITCH_ACTIVE
SAFETY_PROCESS_DENYLISTED
SAFETY_SECRET_REDACTED
CONFIG_INVALID
CONFIG_VERSION_MISMATCH
STORAGE_CF_HARD_CAP_REACHED
STORAGE_DISK_PRESSURE_LEVEL_1
STORAGE_DISK_PRESSURE_LEVEL_2
STORAGE_DISK_PRESSURE_LEVEL_3
STORAGE_DISK_PRESSURE_LEVEL_4
```

(plus any not yet added in M1-M4)

---

## Work-items (PR-sized, ordered)

### Block A — overlay + tray + UX (work-items 1-5)

| # | Title | Acceptance |
|---|---|---|
| 1 | `feat(overlay): egui transparent always-on-top window w/ telemetry tail` | overlay launches via `synapse-mcp overlay`; renders capture fps + reflex list at 30 fps; ≤ 100 MB RSS |
| 2 | `feat(overlay): subscribes to /metrics endpoint of running daemon` | overlay updates ≤ 100 ms after daemon emits metric change |
| 3 | `feat(mcp): tray icon w/ pause/resume/disable-reflex/open-logs/quit` | tray menu items work; hover shows session count + profile; `--no-tray` opt-out functional |
| 4 | `feat(mcp): setup wizard (interactive + --non-interactive)` | fresh machine: wizard completes; `%APPDATA%\synapse\config.toml` + `token.txt` + `agreement.json` written |
| 5 | `feat(safety): bearer token generation + ACL on token.txt` | new install ⇒ token created; ACL restricts to SYSTEM + current user; rotation via `synapse-mcp token rotate` invalidates existing sessions |

### Block B — VLM + describe (work-items 6-7)

| # | Title | Acceptance |
|---|---|---|
| 6 | `feat(models): Florence-2-base ONNX loader + first-call download consent prompt` | first call w/o model ⇒ `MODEL_NOT_LOADED`; operator opts in ⇒ download w/ sha verify; subsequent calls succeed |
| 7 | `feat(mcp): describe tool (05 §3.3)` | bench: `describe(detail="standard")` p99 ≤ 500 ms VLM-dependent (10 §12); returns `description` + `model_id` + `latency_ms` |

### Block C — profiles + assets (work-items 8-10)

| # | Title | Acceptance |
|---|---|---|
| 8 | `feat(profiles): factorio + discord + slack + file_explorer + <one_fps> + roblox_studio TOMLs` | each parses; profile_list shows all; bundled assets (HUD templates, if any) present |
| 9 | `feat(profiles): inert unknown-scope profile templates` | parse; `use_scope = "unknown"`; keymap empty; mode pixel_only; reviewed-profile comment present |
| 10 | `feat(profiles): smoke tests per profile (13 §10)` | each bundled profile passes its smoke test locally |

### Block D — perf + soak (work-items 11-13)

| # | Title | Acceptance |
|---|---|---|
| 11 | `bench: all tracked benches green on reference machine (10 §2 + 13 §7)` | criterion runs all benches; results posted; none regress > 20% vs M4 baseline |
| 12 | `test(soak): 8h synthetic workload (13 §12)` | mem growth ≤ 50 MB; no deadlocks; p99 latencies stable; DB respects soft caps |
| 13 | `feat(perf): spike check (10 §15) — emit `synapse-performance-degraded` if > 2× p99 for > 5s` | injected stall test ⇒ event emitted + `health.subsystems.X.status="degraded_latency"` |

### Block E — observability (work-items 14-15)

| # | Title | Acceptance |
|---|---|---|
| 14 | `feat(telemetry): Prometheus /metrics endpoint (12 §4.3)` | `curl http://127.0.0.1:9100/metrics` returns text format; cardinality bounded per `12 §4.2` |
| 15 | `feat(dashboards): 5 Grafana dashboards JSON committed to dashboards/` | import each; renders against a running daemon producing metrics |

### Block F — installer + signing (work-items 16-19)

| # | Title | Acceptance |
|---|---|---|
| 16 | `chore(release): WiX MSI build (synapse-mcp + synapse-overlay + bundled assets + ViGEmBus checkbox)` | MSI installs cleanly on fresh Win11; uninstalls cleanly |
| 17 | `chore(release): code signing via signtool (SHA256 + timestamp)` | `synapse-mcp.exe`, `synapse-overlay.exe`, `SynapseSetup-x.y.z.msi` all signed; verifiable via `signtool verify /pa` |
| 18 | `chore(release): release.ps1 orchestrating build → sign → bundle → upload` | `pwsh scripts/release/release.ps1 -Version 1.0.0` produces all artifacts |
| 19 | `chore(release): winget manifest submission + crates.io publish for library crates` | winget PR opened; `cargo publish` succeeds for `synapse-core`, `synapse-storage`, ... |

### Block G — docs + license (work-items 20-22)

| # | Title | Acceptance |
|---|---|---|
| 20 | `docs: USER_GUIDE.md (operator quick-start + troubleshooting + profile authoring)` | new user follows guide to working install ≤ 5 min |
| 21 | `chore: THIRD-PARTY-LICENSES.md via cargo about; included in MSI` | all deps listed; license SPDX permitted per `14 §14` |
| 22 | `chore: CHANGELOG.md v1.0.0 + schema lock SCHEMA_VERSION="1"` | post-v1 schema changes blocked by local schema-version checks referencing this version |

### Block H — manual test plan (work-item 23)

| # | Title | Acceptance |
|---|---|---|
| 23 | `test(manual): release gate checklist per 13 §15 signed off by maintainer` | all 5 manual steps pass on a fresh VM; sign-off recorded in CHANGELOG entry |

---

## Acceptance gates (block v1.0 release)

```
✓ M5 demo passes on fresh Win11 VM (5-app scenario, ≤ 30 K tokens total)
✓ All M0-M5 demos still pass on `main` (regression gate)
✓ Soak 8 h clean (13 §12)
✓ All perf budgets met on reference machine (10 §2 / §3 / §4 / §12)
✓ Local configured-host checks and manual FSV green on the release candidate (15 §10.3)
✓ Manual test plan signed off (13 §15)
✓ PRD docs internally consistent (`scripts/check_docs.ps1 -CheckAnchors` green)
✓ cargo deny check clean (14 §14)
✓ No unsafe outside synapse-capture / synapse-hid-host / firmware/pico-hid
✓ No unwrap() outside test code
✓ Crash dumps land on intentional panic (12 §9)
✓ MSI installs + uninstalls cleanly on fresh Win11 VM
✓ MSI signed; signtool verify /pa passes
✓ At least 10 bundled profiles parse + smoke-test pass
✓ All bundled profiles + all default tool params resolve to `Natural` curves + `Natural` keystroke dynamics (OQ-004 DECIDED); no `Instant`/`Burst` defaults shipped — verified via schema-defaults snapshot + profile-validation test
✓ Grafana dashboards import cleanly + render
✓ describe VLM downloads + runs on first call
✓ Token cost: M5 demo ≤ 30 K tokens (15 §7 demo criterion)
✓ Install size ≤ 120 MB (14 §15)
```

---

## Risks (`15 §9` + extras)

| Risk | Mitigation |
|---|---|
| MSI signing cert availability | Self-sign at v1.0 ⇒ SmartScreen "Verified Publisher" prompt once; document; EV cert tracked as fundraising/community workstream |
| VLM bundle size | NOT bundled per `OQ-008`; download on first use w/ consent; `MODEL_NOT_LOADED` until present |
| `<one_fps>` profile choice (free game TBD) | Decision deferred to start of M5; bundled stub if no good free FPS lands |
| Roblox runtime profile scope varies per experience | Roblox Studio = `operator_owned_test`; player runtime = `unknown`; operator picks profile manually |
| Reproducible builds incomplete (`14 §13`) | PE timestamps + COFF section ordering vary on Win; document; post-v1 work |
| Whisper-base bundle decision (`OQ-014`) | Default Whisper-tiny-int8 at M5; revisit if disk-size budget permits |
| Sled vs RocksDB final (`OQ-001`) | Default RocksDB unless > 2 RocksDB crashes traced during soak; flip via feature flag if needed |

---

## Out of scope at M5 (v1.x or v2)

- AI-driven profile authoring
- Cloud-hosted Synapse-as-a-service
- Multi-machine orchestration
- Linux / macOS (v2; `15 §8`)
- Per-game fine-tuned detection (v1.x)
- Visual replay viewer (v2)
- Profile marketplace + signing (v2; `OQ-007`)
- Steam Audio HRTF (v2; `OQ-021`)
- PIO USB host on RP2040 (v2; `09 §12`)

---

## Definition of Done

v1.0.0 cut when:

1. M5 demo passes on a fresh Win11 VM (the 5-app scenario per `15 §7`, ≤ 30 K tokens total).
2. Every acceptance gate above green; **manual FSV with source-of-truth read-back on every row** of `13 §15`.
3. Soak 8 h clean — no memory growth > 50 MB, no deadlocks, no held-key leaks after `release_all`, p99 latencies stable across the run.
4. Manual happy-path + edge-case table filled in by operator in the v1.0.0 release PR (mirror the M2 §9 structure; expand to cover the 5-app M5 scenario + installer + setup wizard + overlay + VLM describe).
5. `CHANGELOG.md` updated; tag `v1.0.0` published with all release artifacts (MSI, portable zip, pico-hid `.uf2`, source tarball).
6. `cargo publish` for library crates; winget manifest PR opened.

**FSV reminder for v1.0:** the installer test reads back via `Get-Package`/`Get-WmiObject` to confirm the MSI registered; the setup wizard test reads back the actual files written to `%APPDATA%\synapse\` and asserts contents byte-by-byte; the overlay test screenshots the window and asserts FPS counter present and incrementing; the `describe` test downloads the model, sha256-verifies it on disk, then calls the tool and asserts a non-empty `description` field. No row is "ok by inspection."

Post-v1 work tracked in `15 §8` (v1.x patches + v2 horizons) and `16_open_questions.md` (remaining OQs).
