# Implementation Plan: Command Palette

**Branch**: `016-command-palette` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/016-command-palette/spec.md`

## Summary

Add a global Cmd/Ctrl+K command palette that lets an operator fuzzy-search and jump to (a) a
currently-loaded cluster resource by name (optionally `kind:name`-scoped), (b) a namespace, or
(c) a sidebar navigation section — all pure frontend, reusing existing store actions
(`openInspector`, `selectNamespace`, `navigate`) and existing UI primitives. The project already
has `cmdk` as a dependency and a ready-built `CommandDialog`/`Command*` wrapper
(`ui/src/components/ui/command.tsx`) that is otherwise only used inside `Picker.tsx`'s attached
combobox variant — this feature is the first consumer of `CommandDialog` as a global overlay.
No backend/protocol changes.

## Technical Context

**Language/Version**: TypeScript (strict), React 18+ — frontend only, no Rust changes

**Primary Dependencies**: `cmdk` 1.1.1 (already a dependency, provides built-in fuzzy scoring via
`Command.Item`), existing `ui/src/components/ui/command.tsx` (`CommandDialog`, `CommandGroup`,
`CommandItem`, etc.), existing Zustand store (`ui/src/store/index.ts`), existing resource
descriptor registry (`ui/src/resources/index.ts`, `descriptorFor`)

**Storage**: N/A — no new persisted state; the palette's open/closed state and query text are
transient React state, not store/persisted state

**Testing**: `pnpm -C ui test` (Vitest) for the search-index/ranking unit logic (constitution III:
"UI logic that decides what the operator sees ... search ... MUST have unit tests"), Playwright
e2e (`ui/tests/e2e/us1-command-palette.spec.ts`) against the existing scripted fake backend

**Target Platform**: Desktop (Tauri v2, Linux/macOS/Windows) — same as the rest of the UI

**Project Type**: Desktop app (Tauri v2 + Rust backend); this feature touches only `ui/`

**Performance Goals**: Palette open and first keystroke-to-results MUST feel instant (no visible
delay) per SC-003; matching over currently-loaded rows (bounded by what's already in `lists`, not
a fresh cluster-wide fetch) keeps this cheap without new indexing infrastructure

**Constraints**: No backend/protocol changes (`src/protocol/v1/` untouched); results MUST be
capped (FR-007) to keep the list scannable and re-render cheap on every keystroke; must not
regress idle CPU (constitution V) — the global keydown listener is a single passive listener,
mounted once, mirroring the existing `useEscapeToClose` pattern in `Inspector.tsx`

**Scale/Scope**: One new global overlay component, one new keyboard-shortcut hook, one new
search-index/ranking module (pure functions, unit-tested), no new store slice — extends the
existing store with a handful of new UI-only actions if needed, wired into existing shell chrome

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Principle I (Operator-First, Explicit Mutation)**: The palette performs navigation only
  (opening the Inspector, changing the namespace filter, changing the active nav section) — it
  never mutates the cluster and never bypasses any existing mutation confirmation flow (FR-014).
  **PASS.**
- **Principle II (One Contract, Backend Owns the Cluster)**: No protocol changes; the palette
  reads only data already present in the frontend store (`lists`, `session`, `NAV_GROUPS`) and
  calls existing store actions (`openInspector`, `selectNamespace`, `navigate`) rather than
  introducing a new backend-facing path. **PASS.**
- **Principle III (Tested at the Boundaries)**: The palette's fuzzy-match/ranking/`kind:name`
  parsing logic gets dedicated Vitest unit tests (this is exactly the "search ... MUST have unit
  tests" case named in the constitution); the end-to-end open→search→select→Inspector flow gets a
  focused Playwright spec against the scripted fake backend, following the existing
  `us<N>-<feature>.spec.ts` convention. **PASS.**
- **Principle IV (Density Scales With Width)**: The palette is a centered overlay dialog (reusing
  `CommandDialog`'s existing responsive `Dialog` styling), not a persistent pane, so it does not
  affect the pane/column density rules; it must remain fully keyboard-operable (FR-008) and touch
  works via existing `Dialog`/`Command` primitives. **PASS.**
- **Principle V (Fast and Quiet)**: One passive `keydown` listener mounted once (no polling);
  search runs only while the dialog is open and only over already-loaded rows, not a new
  cluster-wide watch or poll. **PASS.**
- **Principle VI (User Data Is Sacred)**: No new persisted state, no new credentials/config
  surface. **PASS.**
- No violations requiring the Complexity Tracking table.

## Project Structure

### Documentation (this feature)

```text
specs/016-command-palette/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
└── tasks.md             # Phase 2 output (/speckit-tasks)
```

No `contracts/` directory: this feature adds no protocol commands/events and no new public
interface beyond the React component itself, so there is nothing to contract-document beyond
`data-model.md`.

### Source Code (repository root)

```text
ui/src/
├── components/
│   ├── shell/
│   │   └── CommandPaletteHost.tsx   # NEW — mounts the palette + global Cmd/Ctrl+K listener,
│   │                                 # wired in alongside TopBar.tsx in the app shell
│   └── CommandPalette.tsx           # NEW — the CommandDialog-based palette UI: input, grouped
│                                     # results (Resources / Namespaces / Sections), keyboard nav
├── resources/
│   └── paletteIndex.ts              # NEW — pure functions: build searchable entries from
│                                     # `lists` (via `descriptorFor`) + NAV_GROUPS + namespace
│                                     # list; parse `kind:name` query syntax; rank matches
└── store/
    └── index.ts                    # UNCHANGED — palette calls existing openInspector,
                                     # selectNamespace, navigate actions directly; no new slice

ui/tests/
├── unit/
│   └── paletteIndex.test.ts        # NEW — Vitest: query parsing, kind-scoping, ranking,
│                                     # result capping, empty-state cases
└── e2e/
    └── us1-command-palette.spec.ts  # NEW — Playwright: open via Cmd/Ctrl+K, fuzzy-search a
                                      # known resource, select it, assert Inspector opens on that
                                      # exact resource (follows existing us<N>-*.spec.ts + helpers.ts
                                      # scripted-fake-backend convention)
```

**Structure Decision**: Single-project desktop app structure (existing); this feature is entirely
within `ui/src/`. No new store slice — `paletteIndex.ts` holds pure, unit-testable
data-transformation functions (reading `lists`/`NAV_GROUPS`/namespace state via selectors) and the
two new components are thin UI wiring over them and over existing store actions
(`openInspector`, `selectNamespace`, `navigate`). This mirrors how `Picker.tsx` composes the same
`ui/src/components/ui/command.tsx` primitives without needing its own store slice.

## Complexity Tracking

*No constitution violations requiring justification.*
