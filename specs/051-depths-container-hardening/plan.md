# Implementation Plan: Depths container hardening

**Branch**: `050-depths-stack-manager` | **Date**: 2026-10-24 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `specs/051-depths-container-hardening/spec.md`

## Summary

Rate every container `ok` / `root` / `escape` from a batched `docker inspect` plus the daemon's
security options, show the rating as chips (container rows, console picker, stack list), guard shell
opens with a pure policy function evaluated on a fresh inspect at open time (fail closed), harden the
exec argv, and lint compose files for the same findings on edit and before `up`, with an optional
enforce policy and an audited admin acknowledgement.

## Technical Context

**Language/Version**: Rust 2024 (stable), TypeScript 5 / React 19
**Primary Dependencies**: existing only: Axum/Tower, serde, ts-rs; `serde_yaml` (already used for compose) for the lint
**Storage**: none new (audit log from 050)
**Testing**: `cargo test -p depths-core` (rating, policy, lint tables), `cargo test -p depths-runtime-cli` (inspect parsing, argv), `cargo test -p depths-server` (guard + sweep), vitest, one Playwright project (`us4-shells`)
**Target Platform**: Linux container on a Docker daemon, rootful or rootless
**Performance Goals**: rating adds one batched inspect per inventory refresh; a shell open adds one single-container inspect (<100 ms locally)
**Constraints**: never rewrite compose files; decision server-side; fail closed when inspect fails
**Scale/Scope**: tens of stacks, hundreds of containers

## Constitution Check

- **I. Operator-first, explicit mutation**: lint never edits files; enforce needs an explicit, audited acknowledgement. PASS
- **II. One contract, backend owns the runtime**: rating and decision live in `depths-core`, exposed through `depths-protocol`; the UI only mirrors. PASS
- **III. Tested at the boundaries**: table tests for rating/policy/lint, server tests for each refusal, sweep rows, Playwright for chips and refusal text. PASS
- **V. Fast and quiet**: batched inspect, no polling added. PASS
- **VI. User data is sacred**: no file writes. PASS

## Project Structure

### Documentation (this feature)

```text
specs/051-depths-container-hardening/
├── spec.md  plan.md  research.md  data-model.md  quickstart.md  tasks.md
├── contracts/protocol.md  contracts/config.md
└── checklists/requirements.md
```

### Source Code

```text
crates/depths-core/src/security.rs        # Level, Reason, Profile, profile(), DaemonMode, ShellPolicy, shell decision
crates/depths-core/src/compose_lint.rs    # lint findings from a parsed compose file
crates/depths-runtime-cli/src/lib.rs      # batched security inspect, daemon mode
crates/container-runtime/src/session.rs   # hardened exec argv
crates/depths-protocol/src/{cmd,event,types}.rs   # ratings on containers, lint findings, ack flag
apps/depths/server/src/{config,streams,ws}.rs     # policies, guard at open, end shell on recreate, audit
apps/depths/web/src/features/…            # SecurityChip, picker/stack list chips, lint panel, ack dialog
docs/guides/depths/security.md
```

## Complexity Tracking

None.
