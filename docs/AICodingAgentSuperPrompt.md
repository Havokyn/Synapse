# Synapse Agent Wake-Up Pointer

The full internal agent doctrine is intentionally not tracked in this public
docs tree. It was removed from `docs/` in commit `390cfe4` when internal
planning/specification documents were moved out of the public repo.

Authoritative wake-up sources for agents on this configured host:

1. `AGENTS.md` at the repo root.
2. `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`.
3. Open GitHub issues and closed `type:decision` / `type:context` issues,
   especially #351.

Manual FSV remains mandatory. Do not substitute scripts, tests, benchmarks,
harnesses, CI, GitHub Actions, direct HTTP helpers, or direct storage writes for
real Synapse MCP tool triggers when a Synapse MCP tool exists. If the configured
Codex `mcp__synapse` Streamable HTTP transport is closed, stale, or missing,
treat that as local host setup work and repair the Codex HTTP entry plus bearer
token environment before accepting Synapse runtime behavior. Re-run
`scripts/synapse-setup.ps1` if the standard Codex launchers lack the Synapse
token loader. If the already-running Codex process started before
`SYNAPSE_BEARER_TOKEN` existed or changed, Windows cannot update that process
environment after the fact; do not claim direct `mcp__synapse` FSV is available
until a fresh Codex process initializes with the token loader and the live tool
call succeeds.
