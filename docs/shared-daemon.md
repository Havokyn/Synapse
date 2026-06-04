# Shared Synapse daemon (one DB, many agents)

Synapse stores perception/action state in a single RocksDB, which permits
**exactly one writer process**. To let many agents (Claude Code, Codex, Claude
Desktop, WSL clients) use Synapse at once without each opening — and fighting
over — that database, Synapse runs as **one shared daemon** that owns the DB,
and every client connects to it.

> Do **not** run multiple `--mode stdio` servers against the same `--db`. The
> second one fails to open RocksDB (`STORAGE_LOCK_CONTENDED`) and, before this
> model existed, such duplicates leaked and accumulated. Use the daemon + the
> client modes below instead.

## Components

| Mode | Command | Role |
|------|---------|------|
| `http` (daemon) | `synapse-mcp --mode http --bind 127.0.0.1:7700 --db <path>` | The single owner of the DB. Serves all clients over Streamable HTTP. |
| `connect` (bridge) | `synapse-mcp --mode connect --bind 127.0.0.1:7700` | Thin stdio↔HTTP bridge for stdio-only MCP clients. Auto-spawns the daemon if it is down. |
| `doctor` | `synapse-mcp --mode doctor [--kill-stray]` | Enumerate/classify synapse-mcp processes; clean up leaked orphans. |

A bearer token is read from `%APPDATA%\synapse\token.txt` (or the
`SYNAPSE_BEARER_TOKEN` env var). Clients and the bridge authenticate with it.

## How each client connects

- **Claude Code and Codex** speak Streamable HTTP natively — point the
  `synapse` MCP entry at `http://127.0.0.1:7700/mcp` with the bearer token
  (transport `http` / `streamable_http`).
- **Claude Desktop on Windows** is a stdio client — launch `synapse-mcp --mode
  connect --bind 127.0.0.1:7700`. The bridge forwards JSON-RPC to the daemon
  and streams notifications back; it auto-spawns the daemon on first use and
  forwards its `--db` to the spawned daemon.
- **WSL agents must not launch the Windows `.exe` bridge directly.** Direct
  WSL interop makes the Windows parent look like long-lived `wsl.exe`, not the
  real Linux-side MCP client, so the bridge cannot prove client lifetime. WSL
  Codex/Claude clients should use native Streamable HTTP
  `http://127.0.0.1:7700/mcp` with the local bearer token. A direct WSL
  `--mode connect` launch fails closed with `MCP_CONNECT_UNSUPPORTED_PARENT`.

All clients share one daemon, so they observe the same live world state and
receive the same `subscribe` events.

## Single-instance guarantee

The daemon takes an OS advisory lock on `<db>/daemon.lock` at startup (before
binding the port or opening RocksDB) and records its PID in `<db>/daemon.pid`.
A second daemon for the same DB logs `MCP_DAEMON_ALREADY_RUNNING` (naming the
holder PID) and exits **3**. The lock is released automatically if the holder
dies, so a crash never wedges future launches. Racing `connect` bridges
therefore converge on exactly one daemon.

Storage is opened **eagerly at startup**: a lock conflict logs
`STORAGE_LOCK_CONTENDED` and any other open failure logs `STORAGE_OPEN_FAILED`;
both exit **4** instead of failing later inside a tool call.

## Lifecycle / no orphans

- The `connect` bridge exits when its stdin closes (client disconnect) and also
  arms a **parent-death watchdog** (`WaitForSingleObject` on the parent) so it
  can never outlive the client even on an abrupt Windows kill.
- The `connect` bridge refuses direct WSL interop parents (`wsl.exe` /
  `wslhost.exe`) because that parent belongs to the WSL host, not the real
  Linux MCP client. This is a hard failure with structured log code
  `MCP_CONNECT_UNSUPPORTED_PARENT`; use HTTP transport from WSL instead.
- The spawned daemon is created with `bInheritHandles=FALSE`, so it never holds
  a client's stdio pipe open.
- The daemon stays resident across individual client disconnects and does **not**
  `release_all` global input when one client leaves (only on full daemon shutdown).
- Diagnostic bridge, Codex app-server, browser, Cargo/rustc, and test helper
  processes are temporary. Close them after the check and reread the
  process/socket table so only the intended daemon and requested user-facing
  apps remain.
- `scripts/synapse-setup.ps1` refuses to stop/restart the shared daemon while
  active HTTP MCP sessions, live client TCP connections, or bridge/stdio
  children are present. This fails closed with `SYNAPSE_ACTIVE_CLIENTS_PRESENT`
  and prints the process/socket readback. Use `-ForceRestart` only after a
  deliberate maintenance decision; setup must not silently interrupt another
  agent.
- The Windows auto-start daemon task must not launch through `cmd.exe` at any
  point. `scripts/synapse-setup.ps1` writes
  `synapse-daemon-launch-hidden.vbs` and registers
  `wscript.exe //B //Nologo`; the VBS reads `%APPDATA%\synapse\token.txt`,
  sets process-local auth/log environment, and starts `synapse-mcp.exe`
  directly with hidden window style. The legacy
  `synapse-daemon-launch.cmd` file is deleted so no raw bearer token is
  embedded in a command file. Setup fails with
  `SYNAPSE_DAEMON_CMD_ANCESTOR_FORBIDDEN` if the healthy daemon's process
  lineage still contains `cmd.exe`.

## Health

`GET http://127.0.0.1:7700/health` (with `Authorization: Bearer <token>`) returns
`{ ok, version, pid, uptime_s, subsystems }`. `subsystems.storage.db_path` and
`subsystems.http.active_sessions` confirm which DB the daemon owns and how many
clients are connected.

## Troubleshooting

- **Codex `mcp__synapse` says `Transport closed`** — treat this as configured
  host setup work. Codex should be Streamable HTTP, not stdio: confirm the
  daemon process and `127.0.0.1:7700` socket, confirm
  `SYNAPSE_BEARER_TOKEN` matches `%APPDATA%\synapse\token.txt`, and read
  `codex mcp get synapse`. If it drifted, repair it with
  `codex mcp add synapse --url http://127.0.0.1:7700/mcp --bearer-token-env-var SYNAPSE_BEARER_TOKEN`.
  Re-run `scripts/synapse-setup.ps1` if the standard Windows Codex launchers do
  not contain the Synapse token loader; the setup script patches those launchers
  so future Codex processes load the canonical token before MCP initialization.
  Then re-read health and tool discovery and retry the real wired
  `mcp__synapse` tools. Direct HTTP probes are diagnostics only; do not use
  them as FSV substitutes. If the already-running Codex process was launched
  before `SYNAPSE_BEARER_TOKEN` existed or changed, Windows cannot update that
  process environment after the fact; setup reports
  `SYNAPSE_CODEX_CURRENT_PROCESS_ENV_STALE` and the current chat cannot claim
  direct `mcp__synapse` FSV until a fresh Codex process initializes with the
  token loader.
- **WSL Codex/Claude leaves `synapse-mcp --mode connect` children under
  `wsl.exe`** — this is a configuration error. Reconfigure the WSL client to
  HTTP transport with bearer auth. The bridge now refuses direct WSL interop
  with `MCP_CONNECT_UNSUPPORTED_PARENT` so this stale-child class cannot recur
  silently.
- **`RegisterHotKey ... Hot key is already registered` / "leaked or duplicate
  synapse-mcp instance"** — more than one synapse-mcp is running. Run
  `synapse-mcp --mode doctor` to list them; `--mode doctor --kill-stray` removes
  matching bridge/stdio/orphan/unknown processes only after it identifies the
  live lock-holder daemon for the selected `--db`.
- **`STORAGE_LOCK_CONTENDED` on daemon start** — another process holds the
  RocksDB lock for that `--db`. `doctor` will name the holder PID; stop it or
  point the daemon at a different `--db`.
- **Bridge exits immediately / `MCP_DAEMON_SPAWN_FAILED`** — the daemon did not
  become healthy within the spawn timeout. Check
  `%LOCALAPPDATA%\synapse\logs\daemon-launcher.log` and the rotated
  `%LOCALAPPDATA%\synapse\logs\synapse.log.*` telemetry files for launch,
  `STORAGE_*`, or bind errors.
- **Setup/update says `SYNAPSE_ACTIVE_CLIENTS_PRESENT`** — setup saw a live
  daemon with active MCP sessions, established loopback client sockets, or
  bridge/stdio children. Close the client/helper that owns the session, rerun
  setup, and read the process/socket SoT again. If a restart is truly required
  while clients are attached, rerun with `-ForceRestart` only after coordinating
  a maintenance window; this flag exists to make interruption explicit.
