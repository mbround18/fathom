# Implementation Plan: Managed-Cloud Auth Plugins

**Branch**: `035-cloud-auth-plugins` | **Date**: 2026-09-24 | **Spec**: [spec.md](spec.md)

## Summary
Add a backend `cloud_auth` module that detects exec plugins, resolves and runs them with
timeout/allowlist/redaction, caches credentials in memory (single-flight, expiry-aware), and
exposes typed errors. Connect, context probe, and fleet poll obtain credentials through it.
See [research.md](research.md).

## Technical Context
**Language**: Rust 2024, tokio, kube-rs 2.0.1; React/TS UI (generated bindings)
**Storage**: per-context `execPolicy` additive in `~/.fathom/` config; no token persistence
**Testing**: cargo test (fake plugin test bin), protocol round-trips, Vitest, one Playwright spec vs fake backend
**Platforms**: Tier 1 Ubuntu 26.04, Windows 11, macOS 26
**Performance**: idle <2% CPU; plugin spawns per context bounded by token lifetime, not poll rate
**Constraints**: no shell, no TTY, no token logging, no protocol version bump

## Constitution Check
- I Operator-first: errors verbatim (redacted); no auto-login or install: PASS
- II One contract: additive protocol types, generated TS: PASS
- III Tested at boundaries: fake plugin tests, round-trips, e2e: PASS
- IV Density: tooltip/inspector rows only, no new panes: PASS
- V Fast/quiet: cache removes per-30s spawns; timeouts bound work: PASS
- VI User data: additive config, secrets never logged/persisted, same RBAC/context: PASS
Post-design re-check: PASS. Risk: any new crates (`which`, `secrecy`) must be justified.

## Project Structure
```
specs/035-cloud-auth-plugins/{spec,plan,research,data-model,quickstart,tasks}.md, contracts/
src/backend/cloud_auth/{mod.rs,detect.rs,exec.rs,cache.rs,redact.rs}
src/backend/{fleet.rs,context_status.rs,kube.rs}   # switch to cloud_auth credential path
src/protocol/v1/cloud_auth.rs
ui/src/...  context tooltip/inspector rows; store handler
```

## Complexity Tracking
| Deviation | Why | Simpler alternative rejected |
|---|---|---|
| Own exec runner instead of kube-rs built-in | Need cross-client cache, timeout, allowlist, typed errors | kube-rs alone re-spawns per fresh client and yields generic errors |
