# 08 — Supported Use Policy

## 1. Purpose of this doc

This is a **policy** doc, not a technical doc. It defines the use cases Synapse is built to support, the capabilities contributors must keep out of scope, and the operator confirmations required before sensitive local-control features are enabled.

This policy is binding on contributors. PRs that violate it are rejected. Defaults are conservative: Synapse should work well for local computer use, accessibility tooling, research rigs, sanctioned game-AI experiments, QA automation, and single-player game control without quietly enabling unrelated or high-impact behavior.

---

## 2. The single rule

> **Synapse is local computer-control infrastructure. It should help an operator or an explicitly authorized agent see, hear, act on, and react inside software the operator is allowed to automate. It should not add features whose primary purpose is raw manipulation of third-party processes, unsupported device-identity changes, unregistered persistence, or scaled unattended account operation.**

Natural cursor curves, virtual controllers, fast capture, and reflexes are ordinary local-control primitives. They exist for accessibility, QA, game-AI research, local demos, simulation rigs, and single-player play. The physical hardware-HID path is retired by #588/#589; the supported input strategy is software `SendInput` plus software-only ViGEm controller reports.

---

## 2a. Runtime posture (default-permissive)

The active target of Synapse is **full-state verification of general Windows
computer use** — the everyday business and personal apps an operator runs
(Office, Outlook, browsers, Teams/Slack/Zoom, File Explorer, PDF readers,
shells, system apps, and the long tail of unprofiled apps). Games are no longer
the active target; the bundled game profiles remain only as legacy fixtures.

To support that, a **stock daemon is permissive by default**:

- **Any foreground app is actionable**, including apps with no bundled profile
  (`allow_unknown_profile` defaults on). Restore fail-closed scope gating with
  `--restrict-unknown-profile` / `SYNAPSE_RESTRICT_UNKNOWN_PROFILE=1`.
- **`act_run_shell` / `act_launch` permit any command/target by default**
  (`SYNAPSE_ALLOW_SHELL_ANY` / `SYNAPSE_ALLOW_LAUNCH_ANY` default on). Set either
  to `0` to restore the per-target allowlist. Every command/target is recorded
  in `CF_ACTION_LOG` regardless.
- **All non-audio M3 permissions are granted by default** (`READ_AUDIO` still
  requires `--enable-audio`).
- The legacy game world/server `supported_use.*` gate is **off by default** and
  re-armed only with `SYNAPSE_ENFORCE_SUPPORTED_USE=1`.

**Functional safety is independent of this posture and always active:** the
operator panic hotkey `Ctrl+Alt+Shift+P` (release-all + disable reflexes), the
panic-hook `release_all`, input rate limits, and foreground/focus
stabilization. The §4.1 frozen capabilities below also remain frozen. Profiles
flag outward-facing or irreversible actions (sending mail/messages, deleting
files, ending processes, connecting to remote hosts, changing system settings)
via `action.outward_facing_actions` / `action.irreversible_actions` /
`action.high_impact_actions` metadata so the operator/agent confirms them
before dispatch.

---

## 3. Supported contexts

Profiles declare a `use_scope`. The field is descriptive metadata for permission checks and user-facing warnings.

| Scope | Examples | Default posture |
|---|---|---|
| `productivity` | Notepad, VS Code, Chrome, Slack, Discord, terminals, File Explorer | Actions allowed according to normal tool permissions |
| `single_player` | Minecraft Java local worlds, Factorio, Stardew Valley, Skyrim, KSP, OpenTTD | Game actions allowed through software/ViGEm; retired hardware tokens fail closed |
| `operator_owned_test` | QA fixtures, private test servers, local simulators, replay harnesses | Actions allowed when the profile declares the test boundary |
| `sanctioned_research` | University game-AI rigs, AI tournaments, benchmark environments | Actions allowed with explicit profile metadata and operator setup |
| `unknown` | New apps without a reviewed profile | Observation and actions allowed by default (see §2a); pass `--restrict-unknown-profile` to fail closed until a profile is reviewed |

The profile loader rejects unknown `use_scope` values. Bundled profiles must include `use_scope` and a short comment describing the intended environment.
Bundled benchmark profiles must also expose metadata gates such as
`supported_use.local_world_only`, `supported_use.approved_worlds`, and
`supported_use.remote_server_allowed` through `profile_list`. Those metadata
keys are the profile registry's source of truth until a runtime target-policy
checker is added.

---

## 4. Capability boundaries

### 4.1 Frozen capabilities

These capabilities stay disabled unless an ADR explicitly changes the project scope:

1. **DLL injection into any process.** Synapse does not load code into target applications.
2. **Raw process memory read/write tooling** for other processes. Game-provided or app-provided APIs are acceptable when documented by the application owner.
3. **Kernel driver hooks.** Synapse is user-mode only. No `.sys` files in the install.
4. **Graphics-pipeline injection.** Capture uses Windows capture APIs, not injected hooks.
5. **Custom device-identity firmware in release builds.** Bundled firmware uses the Synapse Pico HID VID/PID from ADR-0008 and does not ship unrelated commercial device IDs.
6. **Unregistered persistence.** Synapse identifies itself plainly in process names, logs, metrics, and device identity strings.
7. **Automatic escalation based on foreground app.** A profile match may select defaults, but it must not silently elevate permissions beyond the operator's startup configuration.

### 4.2 Sensitive but supported capabilities

These capabilities ship because they are useful for legitimate local automation. They remain explicit and auditable:

1. **Natural cursor curves and keystroke pacing.** Useful for accessibility, demos, QA, and smoother game control.
2. **Virtual controller reports.** ViGEm is useful for accessibility adapters, simulation rigs, dedicated game-AI research machines, and software-only gamepad testing.
3. **Graphics Capture API and DXGI Output Duplication.** Standard Windows capture paths.
4. **WASAPI loopback audio capture.** Standard Windows audio loopback.
5. **WinEvent / UIA event subscribers.** Standard Windows accessibility APIs.
6. **Chrome DevTools Protocol attachment.** Public browser API when the browser is configured for it.
7. **Filesystem and process watchers.** Standard Windows APIs, subject to redaction and permissions.

---

## 5. Operator responsibility

By installing and configuring Synapse, the operator acknowledges:

- They are responsible for using automation only in environments where they have authorization.
- Synapse can move input devices, type text, launch processes, read visible content, and store replay logs.
- Sensitive capabilities are opt-in and logged.
- The project provides tooling and safety defaults, not legal or organizational approval for a specific deployment.

First-run prompt:

```
Synapse is a local computer-control tool. By continuing you confirm:

1. You will use Synapse only where you are authorized to automate.
2. You understand Synapse can type, click, launch processes, capture visible
   screen content, capture system audio, and store replay logs.
3. You will enable sensitive capabilities only when they are needed for your
   local workflow, research rig, accessibility setup, QA environment, or
   single-player/sanctioned game profile.

Type 'i agree' to continue. (Decline by closing this prompt.)
```

Acknowledgment is recorded in `%APPDATA%\synapse\agreement.json` with a hash of the prompt text and a timestamp. A new major version may invalidate the previous acknowledgment.

There is no separate hardware-HID first-use confirmation after #589 because
there is no physical HID runtime path. The legacy `hardware` backend token
fails closed with `ACTION_BACKEND_UNAVAILABLE`.

---

## 6. Permission responses

When an action is about to fire, the MCP layer checks session permissions, profile metadata, backend availability, and startup flags before dispatch.

| Situation | Default behavior | Operator override |
|---|---|---|
| `use_scope = "unknown"` / no profile and a write/action tool is requested | Allowed by default (§2a), log event | Pass `--restrict-unknown-profile` / `SYNAPSE_RESTRICT_UNKNOWN_PROFILE=1` to refuse with `SAFETY_PROFILE_ACTION_DENIED` |
| Retired `hardware` backend requested | Refuse with `ACTION_BACKEND_UNAVAILABLE` | Use `software` or `vigem` |
| Audio tool requested without audio enabled | Refuse with `SAFETY_PERMISSION_DENIED` | Start with `--enable-audio` or set `SYNAPSE_ENABLE_AUDIO=true` |
| Launch process / shell command requested | Allowed by default (§2a), recorded in `CF_ACTION_LOG` | Set `SYNAPSE_ALLOW_LAUNCH_ANY=0` / `SYNAPSE_ALLOW_SHELL_ANY=0` to restore the `--allow-launch` / `--allow-shell` allowlist (refuse with `SAFETY_LAUNCH_DENIED_BY_POLICY` / `SAFETY_SHELL_DENIED_BY_POLICY`) |
| Legacy game world/server `supported_use.*` gate | Off by default | Set `SYNAPSE_ENFORCE_SUPPORTED_USE=1` to re-arm |
| Redaction disabled | Requires startup flag and first-use confirmation | `--no-redaction` |
| Non-loopback HTTP bind | Requires startup flag and first-use confirmation | `--bind <addr>` |

The checks gate Synapse's own behavior. They do not inspect or classify third-party protection systems.

---

## 7. Specific guidance for likely v1 profiles

### 7.1 Productivity profiles (active target)

The bundled `productivity` profiles cover everyday business and personal Windows
use and are the **active FSV target**. They prefer accessibility APIs and
semantic invocation over coordinate motion whenever possible:

- **Documents / Office:** Word, Excel, PowerPoint, OneNote, WordPad, Notepad,
  Adobe Acrobat/Reader (PDF)
- **Email & communication:** Outlook, Microsoft Teams, Slack, Zoom
- **Browsers:** Chrome (also Edge/Chromium), Firefox, Internet Explorer
- **System & files:** File Explorer, Windows Settings, Task Manager, Calculator,
  Snipping Tool, Command Prompt, PowerShell, Windows Terminal, Remote Desktop
- **Dev & media:** VS Code, Paint, Photos

Any app without a bundled profile is still fully actionable through the generic
default-permissive path (§2a); a profile adds semantic keymaps, capture tuning,
and FSV verification anchors. Profiles annotate outward-facing/irreversible
actions in metadata so they are confirmed before dispatch.

### 7.1.1 Legacy game profiles (inactive)

The `minecraft.java`, `luanti.minetest`, and `everquest.live` profiles and the
`supported_use.*` world/server gate are retained only as legacy fixtures. They
are **not** the active target and the runtime gate that enforced them is off by
default (§2a). The game-specific subsections below are kept for historical
reference.

### 7.2 Minecraft Java Edition

`single_player`. Recommended first game profile. HUD extraction, keymap, entity detection, and reflex demos target a local world.

### 7.3 Luanti / Minetest Game Benchmark

`operator_owned_test`. The bundled `luanti.minetest` profile is restricted to
the local configured-host benchmark install and approved local benchmark worlds.
Its metadata declares the launch target, benchmark world, and
`supported_use.remote_server_allowed = "false"` so profile-registry/audit FSV
can verify the intended boundary before actions run.

### 7.4 Factorio

`single_player` or `operator_owned_test` depending on setup. Headless support exists through Factorio's own interfaces; Synapse's GUI driving is supplementary.

### 7.5 OpenTTD, BeamNG, KSP, RimWorld, Stardew Valley

`single_player`. Suitable for bundled or community profiles after normal smoke tests.

### 7.6 Browser games and Roblox Studio

Browser games use the Chrome profile machinery. Roblox Studio is `operator_owned_test`. Runtime experiences should start as `unknown` until a profile states the intended environment.

### 7.7 EverQuest Live Evaluation

`operator_owned_test` on the configured host only, with explicit
`supported_use.operator_attended_required = "true"`. The active evaluation
target is the operator-authenticated EverQuest client with an operator-owned
level 1 Dark Elf Wizard on the Frostreaver server, starting in Neriak. The
current acceptance target is reaching level 2 while recording manual
source-of-truth evidence.

The `everquest.live` profile is a foreground-only live-eval candidate. It must
not use unattended loops, process memory access, packet/protocol inspection,
DLL injection, graphics injection, chat/trade/economy/social automation, PvP
automation, or scaled/background account operation. Actions are allowed only
from active operator prompts and visible foreground state. FSV must read the
live process/window/UI, local EverQuest logs/config files where available, and
Synapse action/audit rows before and after each trigger. See
`26_everquest_live_eval.md` and context issue #491.

---

## 8. Updating profile scopes

Changing a bundled profile's `use_scope` is a release-visible change. It requires:

1. A short changelog entry.
2. A profile smoke test update.
3. Documentation of the intended environment in the profile comment.

Synapse does not auto-update profiles without operator consent.

---

## 9. What this doc does NOT cover

- Retired hardware HID design note → `09_hardware_hid_gateway.md`
- Action back-end mechanics → `03_action.md`
- Per-tool permission requirements → `05_mcp_tool_surface.md`
- Redaction, network binding, and local trust boundaries → `11_security_and_safety.md`
