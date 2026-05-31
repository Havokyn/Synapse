# RECOVERY NOTES - Synapse

Resume by:
1. Re-read `docs/AICodingAgentSuperPrompt.md`, `AGENTS.md`, #351, open issue queue, and this `STATE/*` directory.
2. Inspect the existing dirty diff before editing. Treat it as prior single-agent progress, not unrelated user work, unless evidence says otherwise.
3. Continue #589 first. Current code still has live HID surfaces; remove them while preserving fail-closed `Backend::Hardware` parsing/serialization compatibility.
4. After #589, update/close #588 as context if all child work is resolved, then handle #590 and #585.

Do not use GitHub Actions/CI. Do not create FSV scripts or harnesses. For Synapse behavior FSV, prove the real `synapse-mcp` runtime and client-parity tool list before a real tool call, then read the physical SoT separately.
