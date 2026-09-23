# Implementation Plan: Port Forward Interface

**Branch**: `009-port-forward-ui` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/009-port-forward-ui/spec.md`

## Summary

Add the frontend Port Forwards panel: a live list (start/stop/restart/copy-address) plus a
one-click "Forward" action from Pod and Service inspectors with a confirmation step and a
local-port picker with conflict detection. The full backend (`PortForwardManager`, all
commands/events) already ships; this is frontend-only. One bounded prerequisite is added: a
minimal Service list/inspector descriptor (`ui/src/resources/services.tsx`), since the frontend
currently has no Service view at all to host "Forward" from (backend commands `RefreshServices`/
`SelectService` already exist and were simply unused).

## Technical Context

**Language/Version**: TypeScript (strict), React 18, Rust 2024 (no backend logic changes; test
additions only if drift is found)

**Primary Dependencies**: existing stack only — Zustand store, shadcn/ui (`AlertDialog`, `Tabs`,
`Input`, `Button`), `sonner` toast (already used by `HelmReleaseActionsPanel`), `@phosphor-icons/react`. No new npm/cargo dependencies (checked `ui/package.json`).

**Storage**: N/A (backend-owned; UI holds only ephemeral, non-persisted view state)

**Testing**: Vitest (component/unit), Playwright (e2e against the fake backend), `cargo test`
(protocol serde, only if a round-trip test is missing — audit first)

**Target Platform**: Tauri v2 desktop (Tier 1: Ubuntu 26.04, Windows 11, macOS 26)

**Project Type**: Desktop app (Tauri + React frontend, Rust backend) — frontend-only change

**Performance Goals**: Constitution Principle V — list updates within one render frame of the
backend event, no continuous re-render when idle

**Constraints**: FR-013 — no local port-binding/forwarding/persistence logic in the frontend; only
existing protocol commands/events

**Scale/Scope**: Single non-bulk panel, one inspector tab, one shared start/restart dialog, one
port-picker component, plus the minimal Service descriptor prerequisite

## Constitution Check

- **Principle I (explicit mutation)**: `StartPortForward`/`StopPortForward` both go through
  `ConfirmDialog` naming namespace/pod/(container)/port before dispatch; UI never marks a row
  `Active`/`Closed` ahead of the backend's own event (FR-002/FR-004/FR-010). PASS.
- **Principle II (backend owns the cluster)**: only existing protocol commands/events used; no new
  command, no direct `~/.fathom/` access, no local forwarding logic. PASS.
- **Principle III (tested at the boundaries)**: new Playwright specs per user story
  (`us1`–`us4`), unit tests for the port-picker's debounce/conflict logic and the origin-tagging
  merge logic; audit `tests/protocol_serde.rs` for existing coverage of the commands/events this
  feature touches (all pre-exist 009, so likely already covered — verify, add only if missing).
  PASS (verify in Phase 3).
- **Principle IV (density scales with width)**: panel reuses existing responsive list-pane host
  (`PaneHost`) and the inspector's existing tab strip; no new width-class-specific layout code
  needed. PASS.
- **Principle V (fast and quiet)**: list merge (`lists` + `tunnels` + `portForwardOrigins`) is
  computed in the component via `useMemo`, not stored merged, so it doesn't add a `set()` per
  frame; debounced `CheckPortAvailability` (300ms) avoids a command per keystroke. PASS.
- **Principle VI (user data is sacred)**: no `~/.fathom/` format change. PASS.

No violations; no Complexity Tracking entries needed.

## Project Structure

### Documentation (this feature)

```text
specs/009-port-forward-ui/
├── plan.md
├── research.md
├── data-model.md
├── contracts/protocol.md
├── quickstart.md
└── tasks.md
```

### Source Code (repository root)

```text
ui/src/
├── protocol/commands.ts          # SECTION.services, SECTION.portForwards, REFRESH entries, selectCmd('Service', …)
├── resources/services.tsx        # NEW: minimal Service descriptor (prerequisite)
├── resources/index.ts            # register `services` in ALL_DESCRIPTORS
├── store/index.ts                # StoreData.portForwardOrigins + tagging action
├── store/handlers/list.ts        # tag portForwardOrigins on PortForwardsUpdated (small addition)
├── components/shell/PaneHost.tsx # ListBody: render PortForwardsPanel for SECTION.portForwards
├── components/Inspector.tsx      # "Forward" tab for Pod/Service kinds
├── components/panels/PortForwardsPanel.tsx       # NEW: live list, stop/restart/copy/auto_start toggle, empty state
├── components/panels/PortForwardActionsPanel.tsx # NEW: inspector tab body (port selection)
├── components/panels/PortForwardStartDialog.tsx  # NEW: shared confirm + port-picker dialog
└── components/panels/PortForwardPortField.tsx     # NEW: local port input + conflict indicator

ui/tests/unit/…                   # port-picker debounce/conflict + origin-tagging unit tests
ui/tests/e2e/us1-port-forward-start.spec.ts
ui/tests/e2e/us2-port-forward-manage.spec.ts
ui/tests/e2e/us3-port-forward-port-picker.spec.ts
ui/tests/e2e/us4-port-forward-copy-address.spec.ts

docs/MVP.md                       # status row update (after implementation)
```

## Phase 0 / Phase 1

See `research.md` (decisions) and `data-model.md` (state shapes) / `contracts/protocol.md`
(commands/events, test-isolation contract) / `quickstart.md` (validation scenarios).

## Post-Design Constitution Check

Re-checked after Phase 1 design: no new violations introduced by the Service-descriptor
prerequisite (it is read-only, no new actions, no protocol change). PASS.
