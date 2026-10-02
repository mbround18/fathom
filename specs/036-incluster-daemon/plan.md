# Implementation Plan: Optional In-Cluster Daemon

**Branch**: `036-incluster-daemon` | **Date**: 2026-09-24 | **Spec**: [spec.md](./spec.md)

## Summary

Add `fathom-daemon`, a headless read-only Rust binary (new workspace member `daemon/`) that reuses the `fathom` lib's kube/watch code and protocol types to maintain long-lived watches, derive controller-level events, and serve a bounded replayable NDJSON stream plus warm Pod/Event snapshots. The desktop backend gains a per-context, opt-in `daemon` client module that probes via the API server service proxy, streams frames, merges them into a "While you were away" timeline, and falls back to direct API access on any failure. Packaging reuses the 031 Dockerfile and `k8s/` kustomize tree; testing uses a mock-kube unit suite plus a kind integration script. Decisions in [research.md](./research.md).

## Technical Context

**Language/Version**: Rust edition 2024; TypeScript strict + React for the timeline/status UI.
**Primary Dependencies**: existing `kube` 2 (runtime reflectors), `k8s-openapi`, `tokio`, `serde`, `ts-rs`, `tracing`. New: `axum` 0.8 for the daemon HTTP surface (same version 031 adds; dedupe), `tower-test` (dev-only). No new UI deps.
**Storage**: Daemon in-memory only. Client: additive `~/.fathom/daemon.json` (per-context preference + last cursor).
**Testing**: `cargo test` (derivation diff tables, ring buffer/replay/truncation, coalescing, frame round-trips, probe classification, fallback), protocol round-trips + binding drift, Vitest (timeline merge, staleness), Playwright vs fake backend, `k8s/test-daemon.sh` on kind.
**Target Platform**: Daemon: Linux container (amd64, arm64). Desktop Tier 1 platforms unchanged.
**Project Type**: Multi-crate workspace + one new member `daemon/`, additive `k8s/` addon and overlay.
**Performance Goals**: reconnect replay of 1,000 events under 2 s; topology change visible in <= 5 s; daemon < 250 MiB RSS (incl. topology and metrics ring) and < 2% idle CPU at 5,000 objects; client input handling unaffected (batched 250 ms).
**Constraints**: read-only (the only non-read verb is `create` on `subjectaccessreviews` for per-user checks); zero daemon requests when preference off; no new listeners beyond ClusterIP; reached only via a registered `APIService` (research R1); every response filtered to the caller's RBAC (R10, FR-017); no third-party database bundled (R13); bounded buffers.
**Scale/Scope**: single replica, up to ~10k objects, 8 concurrent streams.

## Constitution Check

- I Operator-first/explicit mutation: PASS. Daemon is read-only; Fathom never installs or removes it (FR-011); errors verbatim.
- II One contract: PASS. Types in `src/protocol/v1/daemon.rs`, bindings generated; daemon HTTP frames use the same types; all cluster access stays in Rust.
- III Tested at boundaries: PASS. Round-trip tests for every variant, Playwright vs scripted fake backend per story, kind integration test; failing-first tests per behavior.
- IV Density: PASS. Timeline is a list pane/section that adds info at wider widths; no horizontal scroll; keyboard/touch operable; states have text, not color only.
- V Fast and quiet: PASS. No polling (stream + backoff probe only), batched/bounded, superseded sessions discarded, idle budget stated.
- VI User data sacred: PASS. Additive `daemon.json`; unreadable file falls back to defaults untouched; no secrets read or logged; no privilege escalation (daemon RBAC is a documented static read-only role; client uses its own credentials).
- Clarification 2026-09-24 impact: per-user filtering satisfies Principle VI (no escalation) in shared clusters; aggregated-API identity keeps credentials out of the daemon; topology/metrics stay inside the bounded/quiet budget of Principle V (in-memory, no polling by the client). Gate: PASS.
- Stack: no replacement of stack components. Gate re-check after design: PASS. Complexity Tracking: none, except the shared-lib egui-gating risk (R3), mitigated in T003.

## Project Structure

```text
specs/036-incluster-daemon/        # this directory
daemon/                            # NEW workspace member
  Cargo.toml
  src/{main.rs,config.rs,http.rs,watchers.rs,derive.rs,eventlog.rs,snapshot.rs,
       identity.rs,authz.rs,topology.rs,metrics_ring.rs,series_store.rs}
  tests/{derive_tests.rs,replay_tests.rs,http_tests.rs}
src/protocol/v1/daemon.rs          # NEW shared types (+ mod.rs, cmd.rs, event.rs edits)
src/backend/daemon_client.rs       # NEW probe/stream/fallback, hooks in kube.rs connect path
src/config.rs                      # additive daemon.json load/save
ui/src/                            # status chip, settings toggle, timeline pane, store slice
ui/tests/                          # Playwright + Vitest
deploy/Dockerfile                  # (031) add fathom-daemon target/stage
k8s/addons/fathom-daemon/          # NEW kustomize base (incl. APIService, SAR create, kube-system auth-configmap read)
k8s/overlays/daemon/               # NEW overlay
k8s/samples/daemon-events.yaml     # NEW event generators
k8s/cluster.sh, k8s/test-daemon.sh # up-daemon, integration test
docs/MVP.md                        # §14 Tier D P4 status update
```

## Phases

- **Phase 0**: research (done), workspace crate skeleton, protocol types.
- **Phase 1**: daemon core (event log, derivation, watchers, HTTP) - US1 server side.
- **Phase 2**: client probe/stream/fallback and UI - US1 + US2.
- **Phase 3**: packaging, kind overlay, integration test - US3.
- **Phase 4**: warm snapshots - US4.
- **Phase 5b (US5)**: per-user authz, topology, metrics ring, UI topology/trends.
- **Phase 5**: docs, perf checks, brochure screenshots (last per project convention).

Ordering note: per-user authz (identity.rs, authz.rs) lands with the daemon core, before any endpoint ships, so no unfiltered path ever exists. Parallelizable across agents: daemon crate (Phase 1), client+UI (Phase 2), k8s packaging (Phase 3) once protocol types (Phase 0) are merged.

## Complexity Tracking

None beyond noted risk R3.
