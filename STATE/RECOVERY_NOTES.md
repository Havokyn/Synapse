# RECOVERY NOTES - Synapse

Resume by:
1. Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, #351, the open issue queue, and `STATE/*`.
2. Treat the old all-clear state as stale. #594 remains the open parent context; #589/#590/#588/#585/#635/#605/#606 are closed with RESOLVED evidence.
3. #606 closed at commit `6975d14` with evidence comment https://github.com/ChrisRoyse/Synapse/issues/606#issuecomment-4587883204.
4. Active issue is #607: `scenario(stress): act_launch fleet - all 30 profiles, foreground incl. console apps`.
   - START comment: https://github.com/ChrisRoyse/Synapse/issues/607#issuecomment-4587884557
   - Issue body requires proving `act_launch` starts/foregrounds every bundled-profile app, with `observe` resolving the app profile; explicitly cover cmd/powershell/Windows Terminal; SoT is foreground HWND/process plus `CF_PROCESS_HISTORY`/`CF_ACTION_LOG`.
   - Edges: app already running, wait-title never matches, launch denied by restrictive policy, rapid relaunch, invalid/empty params.
5. Current #607 implementation patch is in the worktree:
   - `crates/synapse-mcp/src/m4.rs`: launch request details, process-history row encode/key helpers, launch timeout max validation, focused tests.
   - `crates/synapse-mcp/src/server/m4_tools.rs`: `act_launch` writes request audit details and successful spawn rows to `CF_PROCESS_HISTORY`.
   - `crates/synapse-reflex/src/storage.rs`: `storage_put_process_history_rows`.
   - `crates/synapse-mcp/src/m3/audit_retention.rs`: `process_history` retention policy.
6. Readback after compaction:
   - live `profile_list include_inactive=true` reports 29 bundled profiles; `rg --files crates/synapse-profiles/profiles` shows 29 `.toml` profile files. Treat this as the fleet-count SoT despite #607 wording saying 30.
   - installed wired `storage_inspect` still has `CF_PROCESS_HISTORY=0`, proving the pre-fix gap.
7. Next #607 step: launch an isolated repo-built daemon from the patched release binary, prove process/socket/auth/health/strict Inspector `tools/list`, then run manual FSV for `act_launch` happy path, console apps, all locally available/acquirable profiles, and required edge cases. Record any app gap only after all reversible local acquisition/setup is exhausted.

Current #607 resume point as of 2026-05-31T16:15:13-05:00:
- The latest patch also touches `crates/synapse-a11y/src/platform/windows/window.rs` plus `cmd.toml`/`powershell.toml`.
- Root causes fixed in code:
  - console targets now request `CREATE_NEW_CONSOLE`;
  - action audit reads fast foreground metadata instead of a depth-1 UIA subtree snapshot;
  - cmd/powershell profiles include title-specific `WindowsTerminal.exe`/`wt.exe` matches;
  - `focus_window` sends a Windows-only Alt activation nudge before retrying `SetForegroundWindow` under foreground-lock rules.
- Supporting checks already passed for these changes and `cargo build --release -p synapse-mcp` completed after the foreground-lock patch.
- Last isolated daemon PID `37348` on `127.0.0.1:7808` was stopped before the rebuild.
- Resume by starting a fresh isolated HTTP daemon on a new port, then prove auth/health/strict Inspector `tools/list` and storage baseline. Rerun:
  - cmd: `target=cmd.exe`, args `["/k","title synapse-607-cmd-title2 && echo synapse-607-cmd-title2"]`, wait `(?i).*synapse-607-cmd-title2.*`.
  - powershell: `target=powershell.exe`, args `["-NoExit","-Command","$host.UI.RawUI.WindowTitle='synapse-607-powershell-title'; Write-Output 'synapse-607-powershell-title'"]`, wait `(?i).*synapse-607-powershell-title.*`.
  - terminal: `target=wt.exe` with a unique title, wait for that title.
- Required verdict for each: Inspector trigger exits 0 without timeout, `CF_PROCESS_HISTORY` increments with hwnd/title/pid, action audit ok row foreground profile resolves to cmd/powershell/terminal as appropriate, and a separate foreground/window/process SoT read agrees.

Update 2026-05-31T16:29:23-05:00:
- The unstable `CommandExt::show_window` patch was replaced with a Windows-only `CreateProcessW` console spawn path. `cargo fmt --check`, `cargo check -p synapse-mcp`, `cargo check -p synapse-a11y`, and focused launch/process-history tests are green.
- Resume by running `cargo build --release -p synapse-mcp`, stopping stale isolated daemon PID `37952` if still present, and launching a new isolated repo-built daemon on a fresh port (suggest `7810` or later). Then redo the MCP precondition and console FSV.
- The first parallel `cargo test` attempt for `launch_process_history_row_records_spawn_without_env_values` hit `LNK1104` during concurrent linking; rerunning that test sequentially passed.

Update 2026-05-31T17:27:03-05:00:
- Latest code also fixes the existing-window fallback discovered during Chrome/Explorer launch FSV. Existing excluded windows only satisfy `wait_for_window_title_regex` when the window process is compatible with the requested launch target or a known console-host alias.
- Supporting checks are green: `cargo fmt --check`, `cargo check -p synapse-mcp`, `cargo test -p synapse-mcp launch_window_selection -- --nocapture`, and `cargo build --release -p synapse-mcp`.
- Previous final daemon PID `51896` was stopped before the rebuild. Resume by launching a new isolated daemon on a fresh port (suggest `7812`) and rerunning the MCP precondition. Do not reuse the prior `7811` evidence for closure except as defect-discovery history.

Update 2026-05-31T17:56:27-05:00:
- Wake-up context and live queue were re-read again after compaction. Wired `mcp__synapse` health/profile_list/storage_inspect/observe all work; live profile fleet SoT remains 29 profiles.
- Final7 daemon PID `39520` on `127.0.0.1:7813` is still alive but must be stopped before rebuilding because it locks `target\release\synapse-mcp.exe`.
- Slack failure root cause: `act_launch` did not spawn Slack. It failed during supported-use/action preflight while foreground was Acrobat, because that preflight read a depth-1 UIA snapshot and encountered a child `RuntimeId` value of `VT_EMPTY` (`cached RuntimeId had unexpected type EMPTY`). Storage after the failed trigger read `CF_ACTION_LOG=35`, `CF_PROCESS_HISTORY=17`; `Get-Process slack` found no process.
- Patch now in worktree:
  - action launch/scope preflight uses fast foreground readback instead of a UIA tree when only foreground identity is needed;
  - reflex action scope checks use the same fast foreground behavior;
  - UIA snapshot child/raw-supplement node failures mark the tree truncated and log warnings instead of aborting; empty RuntimeId gets a process-local fallback element id.
- Checks after that patch passed: `cargo fmt --check`, `cargo check -p synapse-a11y`, `cargo check -p synapse-mcp`.
- Resume by stopping PID `39520`, rebuilding release, starting a fresh isolated daemon on a fresh port (suggest `7814`), proving process/socket/auth/health/strict Inspector `tools/list`, then retry Slack and continue the #607 matrix.

Do not use GitHub Actions/CI. Do not create FSV scripts or harnesses. For Synapse behavior FSV, prove the real `synapse-mcp` runtime and client-parity tool list before a real tool call, then read the physical SoT separately.

Update 2026-05-31T18:49:37-05:00:
- Post-compaction wake-up has been completed again. Wired `mcp__synapse` health/profile_list/storage_inspect/observe works through the configured client; live queue still has #607 open plus #594/#595-#604/#608-#634.
- #607 final8 manual FSV evidence is already captured on repo-built isolated daemon PID `61024`, bind `127.0.0.1:7814`, DB `.runs\607\launch-fleet-final8-20260531T182322\db`. Strict Inspector `tools/list` succeeded, storage baseline was `CF_ACTION_LOG=0`, `CF_PROCESS_HISTORY=0`, and `profile_list` showed 29 profiles.
- Accepted #607 profile launches: `acrobat`, `calculator`, `chrome`, `cmd`, `everquest.live`, `excel`, `explorer`, `firefox`, `luanti.minetest`, `mstsc`, `notepad`, `onenote`, `outlook`, `paint`, `photos`, `powerpoint`, `powershell`, `settings`, `slack`, `snippingtool`, `taskmanager`, `teams`, `terminal`, `vscode`, `word`, `zoom`. Console foreground/profile readback passed for cmd, PowerShell, and Windows Terminal.
- Host gaps after reversible local work: `iexplore` redirects foreground to Edge (`profile_id=chrome`), WordPad/write binaries are absent on this modern Windows host, and Minecraft Java remains bounded by Microsoft sign-in/license/runtime/world-log SoT. Luanti analogue passed. WordPad/IE profile metadata was patched with evidence-policy/configured-host status strings using Microsoft removed-features docs.
- Edge cases captured: already-running Chrome/VS Code; wait-title no-match; empty target; structurally invalid regex; max timeout `600000`; rapid Notepad relaunch; restrictive policy deny on daemon PID `59732`, bind `127.0.0.1:7815`.
- Current cleanup/finalization steps:
  1. Done: stopped repo-built daemons where possible and stopped `eqgame.exe` plus FSV-owned heavy apps to release memory. PID `59732` and port `7815` are gone; PID `61024` is absent from process/CIM/tasklist/taskkill, but Windows still reports a stale TCP LISTEN row on `127.0.0.1:7814`, so do not reuse that port.
  2. Done: removed agent-created untracked EverQuest artifacts `Logs/` and `eqclient.ini` after verifying they were under `C:\code\Synapse`.
  3. Done: final checks passed: `cargo fmt --check`, `cargo check -p synapse-a11y`, `cargo check -p synapse-mcp`, `cargo check -p synapse-profiles`, bundled profile parse test, `cargo clippy -p synapse-mcp --all-targets -- -D warnings`, `cargo test -p synapse-mcp launch_ -- --nocapture`, `cargo test -p synapse-mcp process_history_has_retention_class -- --nocapture`, `cargo build --release -p synapse-mcp`, and `git diff --check` (line-ending warnings only).
  4. Next: commit with `[skip ci]`, post #607 RESOLVED evidence, close #607, then refresh queue.
