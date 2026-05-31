# Synapse

[![Software Input + M5 Registry/Audit Moat](https://img.shields.io/badge/status-software_input_%2B_M5_registry_audit_moat-blue)](https://github.com/ChrisRoyse/Synapse/issues/588)

Synapse is a Rust MCP server that gives AI agents a local computer-use body: structured perception, action, and low-latency reflexes live in Synapse while the connected model remains the brain. M0-M4 are tagged, the physical HID path is retired by the software-only input decision in issue #588, and the M5 profile-registry / audit-data moat is an active P1 workstream tracked from issue #454.

## Agent Doctrine

Agents working in this repository must follow [AGENTS.md](AGENTS.md). Manual
Full State Verification on the configured Windows host is the shipping gate.
Scripts, tests, benchmarks, GitHub Actions, and CI are supporting evidence only;
they are never FSV.

When a behavior has a Synapse MCP tool, agents must verify the real
`synapse-mcp` runtime before FSV: process or stdio child, bind/socket,
authenticated `health`, initialized MCP session, and `tools/list`. The trigger
must be the real MCP `tools/call`, followed by a separate read of the physical
source of truth such as RocksDB rows, file bytes, UI state, logs, or device
state. Tool return values and `health` are liveness/attempt evidence only.

The active architecture direction in #536 is delta-first reality: Synapse should
feed the agent ordered changes in reality after a baseline snapshot, then
periodically audit the accumulated assumption against full physical reality and
force a rebase when drift is found.

Missing local tools, drivers, models, devices, files, services, account state,
or other prerequisites are acquisition/setup work, not blockers. Agents must
use Synapse/local control as the operator-equivalent host control surface, with
full local computer-control responsibility, plus normal OS, shell, browser,
package-manager, and device-management
workflows, to make the missing thing real and then read the physical source of
truth directly. Ask only before hard-to-reverse external actions.
Do not stop at "missing": Synapse gives the agent the same practical local
host-control ability the operator has at this computer. If the operator could
do it locally, the agent must use Synapse and local host workflows to make it
happen, then inspect the source of truth.
Missing local state creates the next action for the agent and must be made
real; it is not a blocker while reversible host work remains.
Nothing is ever `status:blocked` because a configured-host prerequisite is
absent; the only blockable item is the exact operator-only hard-to-reverse
external action left after every reversible local step is exhausted.
Synapse gives the agent full local computer-control responsibility for this
host. Browser downloads, GUI installers, Device Manager checks, package-manager
installs, model/file generation, firmware flashing, app launching, USB/COM
inspection, and UI inspection are agent-owned work when they are reversible on
this host. A blocker exists only for the exact hard-to-reverse external action
left after that local work is exhausted.

## Status: Software Input + M5 Registry/Audit Moat

M0-M4 are tagged (`v0.1.0-m0` through `v0.1.0-m4`). Issue #588 retires the physical HID strategy: `software` (Win32 `SendInput`) is the default keyboard/mouse backend, `vigem` is the software-only virtual controller backend, and the legacy `hardware` token is retained only as a fail-closed compatibility value that returns `ACTION_BACKEND_UNAVAILABLE`. The M5 profile-registry / audit-data learning loop is active now as P1 product architecture. The live strategic context is [issue #454](https://github.com/ChrisRoyse/Synapse/issues/454); child work is tracked in #455-#470.

The profile-registry / audit-data moat is the compounding loop: profile used -> runtime outcome audited -> quality/compatibility learned -> profile improved -> registry distributes better profile -> more evidence. Agents must treat this as a first-class product surface, not incidental telemetry.

Physical sources of truth for that loop include profile TOML and future registry index/package files, RocksDB `CF_ACTION_LOG`, `CF_REFLEX_AUDIT`, `CF_EVENTS`, `CF_OBSERVATIONS`, `CF_SESSIONS`, and `CF_PROFILES` quality rows, consent/export bundles, and MCP readbacks such as `profile_list`, `profile_quality_refresh`, `storage_inspect`, and future registry/audit tools. Manual FSV must trigger the real runtime surface and then read those physical stores directly; GitHub Actions/CI and automated tests remain supporting evidence only.

Contribution rights, attribution, provenance, licensing, consent, and
revocation semantics for that loop are governed by
[docs/computergames/20_profile_registry_governance.md](docs/computergames/20_profile_registry_governance.md).
The optional shared-registry protocol and moderation boundary are governed by
[docs/computergames/21_profile_registry_protocol.md](docs/computergames/21_profile_registry_protocol.md);
local registry use stays offline-capable and account-free.
The local registry storage model is governed by
[docs/computergames/22_profile_registry_data_model.md](docs/computergames/22_profile_registry_data_model.md),
using namespaced `CF_PROFILES` rows plus small `CF_KV` head pointers.
Package manifests are governed by
[docs/computergames/23_profile_package_manifest.md](docs/computergames/23_profile_package_manifest.md),
which defines the transport metadata, provenance, compatibility matrix,
permissions, and hash validation that registry/install tooling must enforce.

## Tools

The full current tool registry is documented in [docs/computergames/05_mcp_tool_surface.md](docs/computergames/05_mcp_tool_surface.md) and [docs/systemspec/13_mcp_tool_reference.md](docs/systemspec/13_mcp_tool_reference.md). The table below is the M1 starter surface retained for quick orientation.

| Tool | Description | Milestone | Status |
|---|---|---:|---|
| `health` | Reports server version, build, uptime, and subsystem health. | [M0](https://github.com/ChrisRoyse/Synapse/milestone/1) | Done |
| `observe` | Returns the current structured perception snapshot. | [M1](https://github.com/ChrisRoyse/Synapse/milestone/2) | Done locally |
| `find` | Searches accessible elements and detected entities by role/name/query. | [M1](https://github.com/ChrisRoyse/Synapse/milestone/2) | Done locally |
| `read_text` | Reads OCR text from a region or element target. | [M1](https://github.com/ChrisRoyse/Synapse/milestone/2) | Done locally |
| `set_capture_target` | Sets the active primary, monitor, window, or element-window capture target. | [M1](https://github.com/ChrisRoyse/Synapse/milestone/2) | Done locally |
| `set_perception_mode` | Overrides perception mode between auto, a11y-only, pixel-only, and hybrid. | [M1](https://github.com/ChrisRoyse/Synapse/milestone/2) | Done locally |

## Build

Use the current installed stable Rust toolchain. M0 is currently verified with Rust 1.95; the repository intentionally does not pin an older toolchain.

```bash
cargo build --release --workspace
```

The release binary is written to:

```text
target/release/synapse-mcp
```

On Windows the binary name is `synapse-mcp.exe`.

## Input Backends

Synapse currently ships two live input backends:

| Backend | Purpose |
|---|---|
| `software` | Keyboard and mouse through Win32 `SendInput`; default for keyboard, mouse, click, type, aim, drag, scroll, combo, and release-all paths. |
| `vigem` | Software-only virtual Xbox/DS4 controller reports through ViGEmBus; default for pad actions. |

The legacy `hardware` backend token still parses for profile/package
compatibility, but it is not a live backend. Requests that resolve to
`hardware` fail closed with `ACTION_BACKEND_UNAVAILABLE` and guidance to use
`software` or `vigem`.

## Run

For MCP clients, run stdio mode:

```bash
synapse-mcp --mode stdio
```

For issue work that needs a process/socket/log source of truth under repo
control, run the loopback HTTP transport with an isolated DB and log directory:

```bash
SYNAPSE_BEARER_TOKEN=local-token synapse-mcp --mode http --bind 127.0.0.1:7700 --db .runs/issue/db
```

Inspect available flags:

```bash
synapse-mcp --help
```

The pre-wired chat MCP tool is owned by the client process. If it reports
`Transport closed`, read the configured child process, log, and binary hash
SoTs; then use the repo-owned stdio or HTTP runtime path for manual FSV. See
`docs/computergames/25_mcp_runtime_fsv_path.md`.

## Quick Demo

The stdio transport speaks newline-delimited JSON-RPC. A client initializes the server, sends `notifications/initialized`, then calls a tool:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"manual-demo","version":"0.1.0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"health","arguments":{}}}
```

The health payload shape is:

```json
{
  "ok": true,
  "version": "0.1.0",
  "build": "dev",
  "uptime_s": 0,
  "subsystems": {
    "action": { "status": "ok" },
    "storage": { "status": "initializing" }
  }
}
```

`uptime_s` is monotonic, subsystem details vary by enabled runtime surface, and
`build` is `dev` unless a build SHA is injected.

## Configure MCP Clients

WSL-global install:

```bash
cargo install --path crates/synapse-mcp --force
```

Codex user config at `~/.codex/config.toml`:

```toml
[mcp_servers.synapse]
command = "/home/cabdru/.cargo/bin/synapse-mcp"
args = ["--mode", "stdio"]
```

Claude Code user config:

```bash
claude mcp add --scope user synapse -- /home/cabdru/.cargo/bin/synapse-mcp --mode stdio
```

Claude Desktop on Windows:

```jsonc
// %APPDATA%\\Claude\\claude_desktop_config.json
{
  "mcpServers": {
    "synapse": {
      "command": "C:\\\\Program Files\\\\Synapse\\\\synapse-mcp.exe",
      "args": ["--mode", "stdio"]
    }
  }
}
```

After the client loads the server, ask it to call the Synapse `health` tool and confirm the response has the shape shown above.

## Documentation Map

- Product and architecture PRD: [docs/computergames/README.md](docs/computergames/README.md)
- Implementation plan: [docs/impplan/README.md](docs/impplan/README.md)
- Current Rust/dependency decision: [docs/adr/0001-current-rust-and-dependencies.md](docs/adr/0001-current-rust-and-dependencies.md)

## License

Synapse is licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
