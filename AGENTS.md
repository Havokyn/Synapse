# Synapse Agent Doctrine

This repository uses GitHub Issues as the coordination and state surface. Read
the issue queue before changing code, and treat `status:in-progress` issues
assigned to this agent as resumable work after context compaction.

## Non-Excusable FSV Rule

Full State Verification (FSV) must be performed manually by the AI agent. It
must never be delegated to a script, test, benchmark, harness, CI job, GitHub
Action, or any other automated substitute.

For every shipped change, the agent must:

1. Define the Source of Truth (SoT): database/table/key, file path, queue,
   metric, global state, external system record, or UI state.
2. Read the SoT before the trigger.
3. Execute the trigger manually with synthetic inputs whose expected outputs
   are known.
4. Read the SoT again with a separate operation and record the actual state.
5. Manually exercise the happy path plus at least three edge cases, printing
   before and after state for each.

Automated tests, property tests, benchmarks, scripts, and build checks are
supporting regression evidence only. They are not FSV and must not be named or
presented as FSV. Do not add new `*_fsv` tests, FSV harnesses, or FSV scripts.
When Synapse exposes a real runtime surface for the behavior under review,
including MCP tools or daemon endpoints, manual FSV must trigger that real
surface and then inspect the separate physical source of truth/state it
produced. Scripts must not stand in for that runtime trigger or source-of-truth
readback.

## No GitHub Actions / CI Gate

Do not dispatch, wait on, or use GitHub Actions/CI as a shipping gate unless a
later explicit operator decision reverses issue #351. Agent commits pushed to
this repo must include `[skip ci]`.

## Required Wake-Up Context

After compaction or a new session, re-read:

1. `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
2. This file
3. Open and closed GitHub decision/context issues, especially #351
4. `git status` and the active issue comments
