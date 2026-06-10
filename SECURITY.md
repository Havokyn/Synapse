# Security Policy

Synapse gives an AI agent a real local computer-use body: it can move the mouse,
type, press keys, drive a virtual controller, run allowlisted shell commands, launch
processes, capture the screen, and read accessibility trees. Treat it as a
powerful, security-sensitive tool and run it only on machines and accounts where
that level of control is acceptable.

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues,
discussions, or pull requests.**

Instead, report privately:

- Preferred: open a [GitHub private security advisory](https://github.com/ChrisRoyse/Synapse/security/advisories/new)
  ("Report a vulnerability").
- Or email **chrisroyseai@gmail.com** with subject `SECURITY: Synapse`.

Please include:

- A description of the issue and its impact.
- Steps to reproduce, or a proof of concept.
- Affected version / commit, OS, and configuration.
- Any suggested remediation.

You can expect an acknowledgement within a few days. Please give a reasonable
opportunity to investigate and ship a fix before any public disclosure. Reports
made in good faith are welcome and appreciated.

## Scope and operational guidance

Because Synapse is designed to control the local machine, the following are
**expected behavior**, not vulnerabilities, when configured by the operator:

- Sending input (keyboard/mouse/controller) to the focused application.
- Capturing the screen and reading UI Automation / accessibility data.
- Running shell commands and launching processes that the operator has
  explicitly allowlisted.

Genuine security issues include, for example: bypassing the configured
allowlists or safety gates, privilege escalation, authentication bypass on the
HTTP transport, leaking secrets/credentials into logs or stored rows, or remote
code execution beyond the operator's configured surface.

### Hardening recommendations

- Run the HTTP transport bound to loopback (`127.0.0.1`) only, and set
  `SYNAPSE_BEARER_TOKEN` to a strong value.
- Run under a least-privilege user account dedicated to automation.
- Keep allowlists for `act_run_shell` / `act_launch` as narrow as possible.
- Do not expose the server to untrusted networks.
