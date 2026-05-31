# DECISION LOG - Synapse

- 2026-05-31: Established active objective from operator request: complete and resolve all open GitHub issues in `chrisroyse/synapse`.
- 2026-05-31: Queue read found four open issues: #590, #589, #588, #585.
- 2026-05-31: Chose to continue #589 first because the worktree already contains partial hardware-HID removal progress matching the issue comment.
- 2026-05-31: Reconciled post-compaction state: hardware-HID removal is already in local commit `e0e9993`; remaining #589 work is stale systemspec docs plus manual runtime/client-parity FSV.
- 2026-05-31: Cleaned systemspec and PRD/impplan stale live hardware-HID references. Decided #589 FSV must launch a repo-built runtime because existing `synapse-mcp` processes are installed binaries and not proof of the local commit.
- 2026-05-31: Fixed signed profile package manifest test expectation after hardware-metadata removal changed the deterministic signature payload digest.
- 2026-05-31: Completed #589 manual FSV through the repo-built HTTP daemon using official MCP Inspector CLI for strict tools/list/client-parity and real `tools/call`; `CF_ACTION_LOG` deltas proved software happy path and fail-closed hardware/error edges.
- 2026-05-31: Posted #589 RESOLVED evidence comment, closed #589, pushed `828eec2` with `[skip ci]`, and moved active work to #590.
- 2026-05-31: Implemented #590 software-only input fidelity benches for SendInput click and ViGEm pad report timing. Manual FSV used the repo-built MCP daemon, Inspector strict `tools/list`, real `act_press`/`act_click`/`act_pad`/`release_all` tool calls, OS key/button state, XInput state, ViGEm PnP state, and `CF_ACTION_LOG` readbacks. Supporting checks and benches passed locally; commit/comment/close are next.
- 2026-05-31: Pushed #590 commit `e7e5b25`, posted RESOLVED evidence, and closed #590. Then closed #588 as resolved because its concrete follow-ups #589 and #590 are both closed with evidence. Remaining open queue is #585.
- 2026-05-31: Implemented #585 as a long-lived worker-owned UIA MTA thread and migrated runtime call sites to data-returning APIs. Direct Windows `UIElement` APIs now fail closed. Supporting checks and docs pass; manual repo-built MCP daemon FSV is pending.
- 2026-05-31: Completed #585 manual FSV through repo-built `synapse-mcp` PID `43940` on `127.0.0.1:7795` using official MCP Inspector. Happy path and depth0/depth999/invalid-param/concurrent observe edges passed with storage/log/process/UI SoT readbacks. Need amend the pushed commit message to add `[skip ci]`.
- 2026-05-31: Amended/pushed #585 as `0814a41` with `[skip ci]`, posted RESOLVED evidence, closed #585, and verified the live open issue queue returned no rows.
- 2026-05-31: After compaction, re-read required doctrine/state/issues and found the prior all-clear state stale. Live queue now has the #594 whole-body stress campaign plus #595-#635 open. Chose #635 first because it directly stress-tests daemon crash recovery and the UIA MTA concurrency behavior shipped for #585.
- 2026-05-31: Implemented #635 crash-recovery ledger/startup replay for held action inputs. Happy-path manual FSV proved a real Inspector `act_press` crash leaves Shift held and restart with the stable configured ledger path releases it; continuing combo/storage/concurrency/restart edges.
- 2026-05-31: Completed #635 manual FSV happy path plus combo crash, storage-write crash, concurrent observe/find/profile/action/reflex calls, invalid empty-key params, and rapid restart loop. Evidence is in `.runs\635\http-fsv-20260531T1106`; supporting checks/diff/commit remain before closing the issue.
- 2026-05-31: Reran #635 supporting checks after FSV and reviewed the diff; ready to commit/push with `[skip ci]` and close #635 with evidence.
