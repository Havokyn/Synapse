# Synapse

[![M1 Perception MVP](https://img.shields.io/badge/status-M1_perception_mvp-blue)](https://github.com/ChrisRoyse/Synapse/milestone/2)

Synapse is a Rust MCP server that gives AI agents a local computer-use body: structured perception, action, and low-latency reflexes live in Synapse while the connected model remains the brain. M1 ships the local perception tool surface; action, storage, profiles, and game-control surfaces start in later milestones.

## Status: M1

M1 is the local perception milestone: a working `synapse-mcp` binary serves MCP over stdio, exposes the six local tools below, and verifies the perception surface through local manual FSV instead of GitHub Actions. The live tracker is the [M1 milestone](https://github.com/ChrisRoyse/Synapse/milestone/2), with mission context pinned in [issue #86](https://github.com/ChrisRoyse/Synapse/issues/86). The implementation checklist is [docs/impplan/02_m1_perception_mvp.md](docs/impplan/02_m1_perception_mvp.md).

## Tools

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

## Run

For MCP clients, run stdio mode:

```bash
synapse-mcp --mode stdio
```

Inspect available flags:

```bash
synapse-mcp --help
```

The HTTP transport flag is present for the future surface but returns `NOT_YET_IMPLEMENTED` in the local M1 build.

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
  "subsystems": {}
}
```

`uptime_s` is monotonic and `build` is `dev` unless a build SHA is injected.

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
