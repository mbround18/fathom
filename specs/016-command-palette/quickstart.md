# Quickstart: Validating the Command Palette

## Prerequisites

- `pnpm -C ui install` (dependencies already include `cmdk`; no new install needed)
- Dev server: `pnpm -C ui dev`, connected to any cluster (real or the app's scripted fake backend
  via `?scenario=...`, per `ui/tests/e2e/helpers.ts`)

## Manual validation (User Story 1 — resource jump)

1. Launch the app and connect to a context with at least one pod loaded (visit the Pods section
   once so its list is loaded, per this feature's "already-loaded resources" scope).
2. Press `Cmd+K` (macOS) or `Ctrl+K` (Windows/Linux) from anywhere in the app.
   - **Expected**: the command palette overlay opens with an empty, focused search input.
3. Type a few characters of a known pod's name.
   - **Expected**: matching pod(s) appear, grouped under a "Resources" heading, ranked by match
     strength.
4. Type `pod:` followed by the same characters.
   - **Expected**: results are restricted to Pod-kind matches.
5. Press `Enter` on the top result (or click it).
   - **Expected**: the palette closes and the Inspector opens showing that exact pod.
6. Reopen the palette and press `Escape`.
   - **Expected**: the palette closes with no navigation change.

## Manual validation (User Story 2 — namespace jump)

1. Open the palette, type part of a known namespace's name.
   - **Expected**: a namespace result appears, visually distinguished (e.g. under a "Namespaces"
     heading) from resource results.
2. Select it.
   - **Expected**: the palette closes and the active namespace filter updates to that namespace
     (visible in the existing namespace picker in the top bar).

## Manual validation (User Story 3 — pane jump)

1. Open the palette, type part of a known sidebar section's name (e.g. "settings").
   - **Expected**: a section result appears under a distinct heading (e.g. "Sections").
2. Select it.
   - **Expected**: the palette closes and the app navigates to that section.

## Edge cases to spot-check

- Open the palette with no query typed yet — no error, empty/placeholder state.
- Type a query matching nothing — a clear "no results" message, not a blank list.
- Type an unknown `kind:` prefix (e.g. `xyz:foo`) — no resource results, not an unfiltered
  fallback.
- Open the palette before any resource list has loaded — namespace and section search still work.

## Automated checks

- `pnpm -C ui typecheck`
- `pnpm -C ui lint`
- `pnpm -C ui test` — includes `ui/tests/unit/paletteIndex.test.ts` (query parsing, kind-scoping,
  ranking, capping, empty results)
- `pnpm -C ui e2e us1-command-palette.spec.ts` — scripted-fake-backend run of: open via keyboard
  shortcut → type a known resource's name → select it → assert the Inspector shows that resource
