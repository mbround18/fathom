# Implementation Plan: Node Shell

**Branch**: `037-node-shell` | **Date**: 2026-09-24 | **Spec**: [spec.md](./spec.md)

## Summary
Add an on-demand privileged debug pod pinned to a chosen node with an attached terminal, gated by a confirmation and an RBAC pre-check, with layered guaranteed cleanup. Reuse 002's exec/terminal plumbing and 024's gate pattern; add a small backend module `src/backend/node_shell.rs` for pod build/create/watch/delete, an additive protocol surface, and a `NodeShellPanel` reachable from the Node inspector plus settings in Cluster Settings.

## Technical Context
**Language**: Rust 2024 (tokio, kube-rs 2.x with `ws`, k8s-openapi); TypeScript/React (Tauri v2, shadcn, xterm already present).
**Storage**: settings only, additive JSON in `~/.fathom/` per context; sessions in memory.
**Testing**: cargo test (serde round-trips, pod-spec builder, state machine, cleanup paths against a fake API), Vitest, Playwright vs fake backend.
**Platform**: Tier 1 desktop (Ubuntu 26.04, Windows 11, macOS 26).
**Performance**: one create + single-pod watch + attach; no impact on streaming budget.
**Constraints**: no privilege escalation; API server remains authority; no new dependencies.
**Scope**: about 7 commands, 3 events, 1 backend module, 1 panel, 1 settings section.

## Constitution Check
- I Explicit mutation: confirmation names node, ns/pod, image, privileges; states reflect backend events; errors verbatim; orphan deletion confirmed by name. PASS.
- II One contract: all cluster work in Rust; additive protocol; generated TS bindings; drift check. PASS.
- III Tested at boundaries: round-trips per variant, unit tests for pod spec/state machine, E2E per story, each cleanup path tested. PASS.
- IV Density: phone width uses a full-pane terminal, wider widths add inspector and status alongside; keyboard/touch; reduced motion. PASS.
- V Fast and quiet: status events only on transitions; output through the existing bounded ring buffer; stale-session events dropped by session_id; a single-pod watch, no polling loops. PASS.
- VI User data: settings additive with default fallback; no secret reads; kubeconfig identity only; not exposed to MCP. PASS.
Re-check after design: PASS; Complexity Tracking empty.

## Project Structure
```text
specs/037-node-shell/{spec,plan,research,data-model,quickstart,tasks}.md, contracts/protocol.md
src/protocol/v1/{cmd,event,mutation}.rs            # additive variants + classification
src/backend/node_shell.rs                          # build_pod, create, watch, delete, sweep, access checks
src/backend/exec.rs                                # generalize target selection (small change)
src/backend/rbac_access.rs                         # reuse SSAR helper
src/backend/persistence.rs                         # settings section
ui/src/components/panels/NodeShellPanel.tsx        # confirm, progress, terminal, orphan list
ui/src/components/panels/ClusterSettingsPanel.tsx  # settings fields
ui/tests/e2e/node-shell.spec.ts
tools/devbridge/src/guard.rs                       # refuse StartNodeShell in sandbox
```

## Design decisions
- **Session lifecycle** is owned by the backend; the UI is a view of `NodeShellStatus`. Singleton: a replaced session deletes its pod first.
- **Cleanup layers**: explicit delete; exit/context-switch hooks with a 5 s bound; pod `activeDeadlineSeconds`; label-based orphan sweep (terminal-phase pods included); delete treats NotFound as success. Delete uses the client captured at session start so a context switch cannot target the wrong cluster.
- **Failure mapping**: create errors, admission errors, pod conditions, waiting reasons and start timeout map to `Failed` with verbatim detail and trigger cleanup.
- **Dependencies on 024/025**: consume the confirm dialog and SSAR helper; if 024 is unmerged, Foundational tasks add them in the shared locations named there so a later merge reduces to dedupe.
- **Out of scope**: Windows nodes, MCP exposure, image catalog, concurrent shells, sessions surviving app restart.
