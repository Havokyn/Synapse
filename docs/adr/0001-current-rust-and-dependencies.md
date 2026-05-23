# ADR-0001 — Use Current System Rust and Current Dependencies for M0

## Context

The original PRD and issue set described an older Rust setup: edition 2024 with
MSRV 1.83, a repo-local `rust-toolchain.toml`, and older dependency examples.
The operator clarified on 2026-05-23 that the repo should use the current Rust
toolchain already installed on this workstation and current compatible
dependency versions because this machine is already maintained for Rust, CUDA,
and related project dependencies.

## Decision

M0 uses the installed stable Rust system toolchain as the implementation source
of truth. At the time of this ADR, the local toolchain is:

- `rustc 1.95.0`
- `cargo 1.95.0`
- `stable-x86_64-unknown-linux-gnu`

The repo will not create a local `rust-toolchain.toml` pin for M0. Workspace
dependencies use current compatible crates resolved from crates.io, with
`Cargo.lock` providing the resolved reproducible graph.

Compatibility is part of "current": if the newest search result is intentionally
non-compiling or incompatible with the workspace, use the newest usable line and
record the reason in the issue evidence.

`bincode` is excluded from M0 dependencies because `3.0.0` intentionally fails
to compile and `2.0.1` is rejected by RustSec advisory RUSTSEC-2025-0141.

## Rationale

This matches the actual development environment the operator uses across Rust
projects. Copying old dependency versions or forcing an old toolchain would make
the workspace less representative of the real CUDA/Rust setup on this machine.

## Alternatives Considered

- Pin `rust-toolchain.toml` to `1.83.0` — rejected because it conflicts with the
  current environment and Rust 2024 support.
- Pin `rust-toolchain.toml` to `1.85.0` — rejected because the operator wants the
  installed current system toolchain, not another local override.
- Copy dependency versions from `14_build_and_packaging.md` verbatim — rejected
  because those are stale examples relative to current project setup.

## Consequences

- Positive: M0 builds against the real local Rust/CUDA environment.
- Positive: `Cargo.lock` still records exact dependency resolution.
- Negative: stale PRD and issue body wording must be reconciled as M0 moves.
- Trade-off accepted: local reproducibility is based on the active stable
  toolchain plus lockfile rather than a rustup override file.

## References

- GitHub decision issue: #83
- GitHub discovery issue: #82
