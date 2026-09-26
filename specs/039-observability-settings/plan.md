# Implementation Plan: Persist and Hydrate Observability Settings

**Branch**: `039-observability-settings` | **Date**: 2026-09-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/039-observability-settings/spec.md`

## Summary

Make Grafana Save real (persist base URL per context, API key in the OS credential store) and add
one read path, `GetObservabilitySettings`, answered by an `ObservabilitySettings` event carrying
non-secret Prometheus and Grafana settings plus a `grafana_key_set` flag. The Grafana panel and
Prometheus form hydrate from it on open and on context change and show configured indicators.

## Technical Context

**Language/Version**: Rust edition 2024 (backend), TypeScript 5 / React (UI)

**Primary Dependencies**: tokio, serde, ts-rs (generated bindings), `keyring` (feature `os-keyring`, already used by `backend/oidc/store.rs` and `backend/ai/store.rs`), zustand store, Vitest, Playwright

**Storage**: `~/.fathom/integrations.json` (non-secret), OS keyring (Grafana API key)

**Testing**: cargo test (unit, protocol round-trip), Vitest, Playwright fake-mode e2e, `pnpm e2e:live` against kind

**Target Platform**: Tier 1 desktop (Tauri) and hosted server

**Project Type**: desktop app with shared backend and protocol

**Performance Goals**: read path answers in one round trip, no cluster call (SC-004: under 1 s)

**Constraints**: secrets never in the protocol, JSON, or logs; additive on-disk format change

**Scale/Scope**: a handful of contexts per user, two forms, two commands, one event

## Constitution Check

| Principle | Status |
|-----------|--------|
| I. Explicit mutation | Pass. Save remains an explicit user action; a new read-only command is added and registered as non-mutating in `protocol/v1/mutation.rs`. |
| II. One contract | Pass. New command and event live in `protocol/v1`; UI only speaks the protocol; ts-rs types regenerated. |
| III. Tested at boundaries | Pass. Protocol round-trip tests, backend unit tests with an in-memory secret store, fake-mode and live e2e. |
| IV. Density scales | Pass. Existing form layouts gain one indicator line; phone e2e included. |
| V. Fast and quiet | Pass. Read touches disk and keyring only, no polling. |
| VI. User data sacred | Pass with care. Existing `integrations.json` must keep loading (see research R1). Secrets never leave the credential store. |

Post-design re-check: pass.

## Project Structure

### Documentation (this feature)

```text
specs/039-observability-settings/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/observability-settings.md
└── tasks.md            # /speckit-tasks
```

### Source Code (repository root)

```text
src/protocol/v1/cmd.rs, event.rs, mutation.rs        # GetObservabilitySettings, ClearGrafanaKey, ObservabilitySettings event
src/backend/integrations.rs                          # per-context settings map, additive migration
src/backend/secret_store.rs (or reuse oidc/store)    # trait + keyring impl + in-memory test impl
src/backend/cmd/integrations.rs                      # real ConfigureGrafana, read + clear handlers
ui/src/protocol/generated/                           # regenerated ts-rs
ui/src/store/                                        # integrations.settings slice, event handler
ui/src/components/panels/GrafanaPanel.tsx            # hydrate, indicator, key set/unset, clear
ui/src/features/metrics/PrometheusForm.tsx           # hydrate, indicator
ui/tests/e2e/ (fake) and ui/tests/live/              # coverage
```

**Structure Decision**: extend the existing integrations command family and store slice; no new crate or module boundary beyond a small secret-store seam.

## Complexity Tracking

No constitution violations.
