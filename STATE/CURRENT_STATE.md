# CURRENT STATE - Synapse

## 2026-05-31T10:49:28-05:00
- Required wake-up context was re-read after compaction:
  - `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`
  - `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
  - `AGENTS.md`
  - #351 manual-FSV/no-CI decision and context issue/decision lists
  - live open queue and active issue comments
  - `git status`, `git log -10`, and current branch
- Git state readback:
  - branch `main`
  - `git status --short --branch`: `## main...origin/main`
  - `HEAD`: `a3f6c43 docs(state): record all issues closed [skip ci]`
- Prior queue #589/#590/#588/#585 is closed with evidence. #585 implementation commit is `0814a41` and evidence is at https://github.com/ChrisRoyse/Synapse/issues/585#issuecomment-4587147620.
- New live open queue now contains 42 issues: #594 parent context plus #595-#635 child stress/showcase issues, all opened after the prior all-clear state.
- #594 mission: prove every live Synapse MCP tool under load with manual FSV, real MCP `tools/call` triggers, strict client-parity `tools/list`, and separate physical SoT readbacks.
- Active first child: #635 `scenario(stress): crash recovery + concurrent-call thread safety (UIA MTA)`.

## Current Queue Snapshot
- #594 remains parent context and should stay open until all children are resolved or explicitly dispositioned.
- P1 children currently open: #595, #596, #600, #603, #605, #606, #607, #608, #609, #613, #614, #616, #617, #621, #624, #633, #634, #635.
- P2/P3 showcase and breadth children are also open: #597-#599, #601-#602, #604, #610-#612, #615, #618-#620, #622-#623, #625-#632.

## Active Work: #635
- Goal: prove daemon crash recovery leaves no inputs held and concurrent tool calls do not trigger UIA cross-thread errors/panics.
- Required runtime FSV shape:
  - prove repo-built `synapse-mcp` process/bind/auth/health/MCP init/strict `tools/list` before behavior acceptance.
  - trigger held-input, concurrent observe/find/action/reflex/profile paths via real MCP `tools/call`.
  - after each trigger read separate SoTs: OS key/button state, XInput/pad state where applicable, process/socket state, `health`, `storage_inspect`/`CF_ACTION_LOG`, daemon log bytes, and UIA worker diagnostics.
  - happy path plus at least 3 edges: crash during combo/reflex, crash during storage write, concurrent observe during `profile_activate`, rapid restart loop, plus invalid/empty/boundary params where applicable.
- Investigation next: inspect existing action held-state/release-all/panic-hook/startup recovery code and MCP concurrency surfaces, then launch a repo-built daemon on an isolated DB for manual evidence.
- Implementation added an action crash-recovery ledger and startup replay path in `synapse-action`/`synapse-mcp`.
- Supporting checks already passed before runtime FSV: `cargo fmt`, `cargo check -p synapse-action`, `cargo check -p synapse-mcp`, focused `synapse-action` recovery tests, `cargo clippy -p synapse-action -p synapse-mcp --all-targets -- -D warnings`, and `cargo build --release -p synapse-mcp`.
- Manual FSV run directory: `.runs\635\http-fsv-20260531T1106`.
- Repo-built daemon evidence:
  - initial PID `46188`, bind `127.0.0.1:7796`, binary `C:\code\Synapse\target\release\synapse-mcp.exe`.
  - strict MCP Inspector `tools/list` succeeded with 80 tools and required #635 tools present.
  - real `tools/call health`, `profile_list`, `profile_activate`, `storage_inspect`, `act_press`, and `release_all` have been used in the run.
- Happy crash-recovery FSV evidence:
  - SoT before: Shift up, recovery ledger absent, `CF_ACTION_LOG=0`.
  - Trigger: Inspector `tools/call act_press` with `keys=["shift"]`, `hold_ms=30000`, `backend=software`; OS Shift became down and `action_recovery.jsonl` contained a `key_held` row.
  - Forced-kill: stopped PID `46188`; socket closed; OS Shift remained down; ledger still contained the held key.
  - Restart with the same configured recovery-file path: new PID `43200`; startup log `ACTION_CRASH_RECOVERY_READBACK ... after=stale_inputs_released recovered_keys=1`; OS Shift up; ledger removed; `release_all` returned zero held inputs.
- Note: one intermediate restart intentionally showed a setup mismatch, looking under `db\action_recovery.jsonl` while the original daemon used the explicit run-root ledger. Acceptance evidence uses the stable configured ledger path and records that path.
- #635 manual FSV status: behavior evidence captured in `.runs\635\http-fsv-20260531T1106`.
  - Happy path: `act_press` long Shift hold killed at PID `46188`; restart with same ledger path released one stale key and removed the ledger.
  - Edge 1: `act_combo` scheduled a long Shift hold; killing PID `43200` left Shift down and ledger populated; restart released one stale key.
  - Edge 2: `storage_put_probe_rows` was killed while its Inspector client was still running; restart reopened RocksDB, `health` was ok, and `storage_inspect` read `CF_KV=0`, `CF_ACTION_LOG=4` with no corruption.
  - Edge 3: concurrent Inspector clients for `observe`, `find`, `profile_activate`, `act_press`, `reflex_register`, and `storage_inspect` all returned `isError=false`; Shift ended up; `CF_ACTION_LOG` advanced 4->6 and `CF_REFLEX_AUDIT` advanced 1->2; daemon log showed one `A11Y_UIA_WORKER_READY` and zero cross-thread/RPC wrong-thread/panic matches.
  - Edge 4: invalid `act_press keys=[]` returned MCP error `act_press keys must contain at least one key`; Shift stayed up; ledger absent; `CF_ACTION_LOG` advanced 6->8 with `TOOL_PARAMS_INVALID`.
  - Edge 5: three explicit rapid restart cycles succeeded on `127.0.0.1:7796`; final PID `42120`, strict Inspector `tools/list` count 80, `health.ok=true`, Shift up, recovery ledger absent.
  - Log readback across 8 stderr files: `CrossThreadOrPanicCount=0`; only non-hotkey error was the intentional invalid-param response.
- Supporting checks after FSV:
  - `cargo fmt --check`
  - `git diff --check` (exit 0; line-ending warnings only)
  - `cargo check -p synapse-action`
  - `cargo check -p synapse-mcp`
  - `cargo test -p synapse-action recovery_log --lib -- --nocapture`
  - `cargo clippy -p synapse-action -p synapse-mcp --all-targets -- -D warnings`
  - `cargo build --release -p synapse-mcp`
- Diff review completed for the action recovery module, keyboard/mouse/ViGEm ledger hooks, MCP startup recovery hook, Cargo metadata, and state notes.
- Current next step: commit with `[skip ci]`, post #635 RESOLVED evidence, close #635, then continue the open queue.

## Standing Rules
- No GitHub Actions/CI dispatch, waits, or CI-gated claims.
- Commits pushed by this agent must include `[skip ci]`.
- Automated checks/benches can support regression confidence only; they are not FSV.
- Missing local prerequisites are acquisition/setup work, not blockers, until only a hard-to-reverse operator-only external action remains.
