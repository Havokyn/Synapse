# RECOVERY NOTES - Synapse

Resume by:
1. Re-read `docs/AICodingAgentSuperPrompt.md`, `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`, `AGENTS.md`, #351, the open issue queue, and `STATE/*`.
2. Treat the old all-clear state as stale. The live queue read at 2026-05-31T10:49:28-05:00 found #594 plus #595-#635 open.
3. Prior completed work remains closed: #589/#590/#588/#585. #585 evidence: https://github.com/ChrisRoyse/Synapse/issues/585#issuecomment-4587147620; implementation commit `0814a41`.
4. Active next issue is #635: crash recovery plus concurrent-call thread safety for UIA MTA.
5. #635 implementation is in the working tree: `synapse-action` has a crash-recovery JSONL ledger for held keys/buttons/pads and `synapse-mcp` replays it at startup.
6. Manual FSV run directory is `.runs\635\http-fsv-20260531T1106`.
7. Current repo-built FSV daemon should be PID `43200` on `127.0.0.1:7796`, started from `target\release\synapse-mcp.exe` with `SYNAPSE_ACTION_RECOVERY_FILE=C:\code\Synapse\.runs\635\http-fsv-20260531T1106\action_recovery.jsonl`.
8. Happy path evidence is captured: real Inspector `act_press keys=["shift"] hold_ms=30000 backend=software`, forced kill of PID `46188`, OS Shift stayed down, restart with stable ledger path released Shift and removed the ledger (`ACTION_CRASH_RECOVERY_READBACK ... recovered_keys=1`).
9. #635 FSV completed:
   - happy `act_press` crash/restart released held Shift from ledger.
   - `act_combo` scheduled hold crash/restart released held Shift from ledger.
   - storage write crash restarted cleanly and `storage_inspect` opened RocksDB.
   - concurrent `observe`/`find`/`profile_activate`/`act_press`/`reflex_register`/`storage_inspect` all returned `isError=false` and no UIA cross-thread/panic log matches.
   - invalid `act_press keys=[]` failed closed and wrote `TOOL_PARAMS_INVALID`.
   - rapid restart loop succeeded through final PID `42120`, health ok, tools/list count 80.
10. Supporting checks passed after FSV: `cargo fmt --check`, `git diff --check`, `cargo check -p synapse-action`, `cargo check -p synapse-mcp`, focused `synapse-action` recovery tests, clippy for `synapse-action`/`synapse-mcp`, and `cargo build --release -p synapse-mcp`.
11. Resume by committing with `[skip ci]`, posting #635 RESOLVED evidence, closing #635, and moving to the next live open issue.

Do not use GitHub Actions/CI. Do not create FSV scripts or harnesses. For Synapse behavior FSV, prove the real `synapse-mcp` runtime and client-parity tool list before a real tool call, then read the physical SoT separately.
