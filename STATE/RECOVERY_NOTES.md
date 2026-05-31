# RECOVERY NOTES - Synapse

Resume by:
1. Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, #351, the open issue queue, and `STATE/*`.
2. Treat the old all-clear state as stale. #635 is now closed in commit `632a834`; the live queue read after wake found #594 plus #595-#634 open.
3. Prior completed work remains closed: #589/#590/#588/#585/#635.
4. Active issue is #605: `release_all` + panic-hook + stuck-key auto-release safety. START comment: https://github.com/ChrisRoyse/Synapse/issues/605#issuecomment-4587382776.
5. Current key facts for #605:
   - wired `mcp__synapse.health` works but reports `operator_hotkey=unavailable`.
   - process cleanup preserved PID `45712` and stopped leaked sibling `synapse-mcp` processes so an isolated daemon can register Ctrl+Alt+Shift+P.
   - code paths read: `synapse-action/src/safety.rs`, `hotkey.rs`, `emitter/*`, `synapse-mcp/src/m2/release_all.rs`, `server/m2_tools.rs`, `m2/press/live.rs`, and reflex hold controllers.
   - use repo-built HTTP daemon, official MCP Inspector CLI for strict client-parity, and real `tools/call` triggers.
6. #605 defects found and patched:
   - `software::release_all` physically released mouse buttons but did not clear crash-recovery button ledger rows.
   - hold-move raced the action-emitter auto-release timer at exactly 30s; added a 1s reflex safety-cap grace.
   - active hold-button reflexes reasserted mouse buttons after `release_all`; patched `release_all` to disable initialized reflexes first and patched operator disable to stop the scheduler after disabling.
7. Current patched release daemon for manual FSV: PID `50668`, bind `127.0.0.1:7797`, run dir `.runs\605\release-fsv-final2-20260531T130745`, operator hotkey registered, strict Inspector `tools/list` count 80 with #605 tools present.
8. Completed #605 release/hotkey/auto-release/invalid-param evidence on PID `50668`:
   - empty release_all: zero releases, OS up, ledger absent, storage advanced.
   - active button+pad release_all: released 2 buttons and 1 pad; OS/XInput neutral; ledger absent; `reason="release_all"` audit rows.
   - active key release_all: before Shift/Ctrl down with ledger rows; release_all returned `released_keys=2`; after OS up and ledger absent; log cancelled 2 key timers.
   - stuck-key auto-release: before Shift down with ledger; after 32s Shift up, ledger absent, `STUCK_KEY_AUTO_RELEASED`, reflex expired.
   - operator hotkey: before MBUTTON and XInput A held; `act_press ctrl+alt+shift+p` triggered global hotkey; after OS/XInput neutral, ledger absent, hotkey log ok, reflex audit reason `operator_hotkey`.
   - invalid params: `act_press keys=[]` failed with `TOOL_PARAMS_INVALID`, OS neutral, ledger absent, storage action log advanced.
9. Panic-hook debug FSV is also complete. Debug daemon PID `53320` on `127.0.0.1:7798` with run dir `.runs\605\panic-fsv-20260531T132034` proved: before Shift up/ledger absent/storage `CF_ACTION_LOG=0`; real Inspector `act_press keys=["shift"] hold_ms=30000` timed out after forced panic; after Shift up/ledger absent/process alive/health ok/storage `CF_ACTION_LOG=1`; logs include `M2_ACT_PRESS_FORCE_PANIC_AFTER_KEYDOWN`, panic hook `SAFETY_RELEASE_ALL_FIRED reason="panic" result="ok"`, and emitter `released_keys=1 cancelled_key_timers=1`. Debug daemon was stopped and port `7798` closed.
10. Final #605 supporting checks are green: `cargo fmt --check`, package checks for action/reflex/mcp, focused mcp/reflex tests, clippy across touched crates, release build, and `git diff --check` (line-ending warnings only). Diff review is complete.
11. Resume by committing/pushing with `[skip ci]`, posting #605 RESOLVED evidence, closing #605, then continuing the live issue queue.

Do not use GitHub Actions/CI. Do not create FSV scripts or harnesses. For Synapse behavior FSV, prove the real `synapse-mcp` runtime and client-parity tool list before a real tool call, then read the physical SoT separately.
