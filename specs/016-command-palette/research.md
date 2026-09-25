# Phase 0 Research: Command Palette

All items below were resolved by inspecting the existing codebase; no external research or new
dependencies were needed. There were no `NEEDS CLARIFICATION` markers in the spec's Technical
Context to resolve.

## Decision: Reuse `cmdk` + existing `CommandDialog`, add no dependency

**Decision**: Build the palette on the project's existing `cmdk` dependency (`ui/package.json`,
already `^1.1.1`) and the existing shadcn wrapper `ui/src/components/ui/command.tsx`, which
already exports `CommandDialog` (a `Dialog`-backed, `top-1/3`-positioned overlay with hidden
title/description for a11y) plus `Command`, `CommandInput`, `CommandList`, `CommandEmpty`,
`CommandGroup`, `CommandItem`, `CommandShortcut`.

**Rationale**: `cmdk` already provides fuzzy/subsequence scoring for `CommandItem` via its
`value`/`keywords` props, so no separate fuzzy-search library (e.g. Fuse.js) is needed.
`CommandDialog` is purpose-built for exactly this use case (a global Cmd+K overlay) and is
currently *unused* — the only existing consumer of the `Command*` primitives, `Picker.tsx`, uses
the lower-level `Command`/`CommandInput`/`CommandList`/`CommandItem` pieces directly inside a
`Popover` (an attached combobox, not a global overlay), not `CommandDialog`. Using `CommandDialog`
here is both the right fit and validates that piece of the design system for the first time.

**Alternatives considered**:
- *Fuse.js or a custom scorer*: rejected — would duplicate `cmdk`'s built-in matching for no
  benefit, and adds a dependency the constitution asks to avoid without justification.
- *A bespoke `Popover`-based overlay (like `Picker.tsx`'s `SearchPicker`)*: rejected — a global,
  keyboard-invoked overlay is a dialog, not a trigger-attached popover; `CommandDialog` already
  models this correctly (centered, no trigger element, `Escape`/outside-click to dismiss via the
  underlying `Dialog`).

## Decision: No global search index in the store; build an ephemeral index from `lists` on open

**Decision**: The palette does not introduce a new store slice that indexes "all resources."
Instead, when the palette opens (or on each keystroke), `paletteIndex.ts` builds its candidate
list by iterating the store's existing `lists: Record<ListKey, ListState>`, filtering to
`ListKey`s belonging to the currently active context (matching FR-012's "current context only"),
resolving each list's `SectionKey` to its `ResourceDescriptor` via `descriptorFor`, and mapping
`ListState.rows` through `descriptor.ref(row)` to get each row's `ObjectRef` and display name.

**Rationale**: There is no existing flat "all resources" index in the store — resource lists are
loaded lazily per section/scope/namespace as the operator navigates (`ui/src/store/keys.ts`'s
`listKey`). Building one eagerly would mean either (a) proactively fetching every section on app
start, which contradicts the app's lazy-loading design and the constitution's idle-CPU/perf
principle (V), or (b) maintaining a second denormalized copy of every list in a new slice, which
duplicates state the constitution's single-store comment explicitly warns against ("components
subscribe with narrow selectors" over one flat store). Deriving the index on demand from what's
already loaded is consistent with the spec's own Assumptions section ("draws resource results
from data already loaded into the frontend store... not from an on-demand cluster-wide fetch").

**Alternatives considered**:
- *Eager cross-section fetch on palette open*: rejected — violates lazy-loading and performance
  goals (SC-003: palette must open instantly regardless of resource count) and is explicitly
  out of scope per the spec's Assumptions.
- *New persisted/global "resource index" store slice, kept in sync via `ingest`*: rejected as
  unnecessary complexity — no other feature needs this index, and deriving it on demand from
  existing `lists` is cheap (bounded by what's already in memory) and requires no new
  synchronization logic.

## Decision: `kind:name` parsing maps the `kind` token to `SectionKey`/descriptor `kind`, not a new type

**Decision**: A query is parsed into an optional `kind` token (text before a literal `:`) and a
`name` token (remainder, or the whole query if no `:`). The `kind` token is matched
case-insensitively against each loaded section's `ResourceDescriptor.kind` (e.g. `"Pod"`) using
the same fuzzy tolerance as name matching (so `"pod:my-app"` and `"po:my-app"` both work), rather
than requiring an exact kind string.

**Rationale**: Descriptors already carry a canonical `kind` string (`ResourceDescriptor.kind`,
matching the store's detail kind) — reusing it avoids inventing a second kind-name vocabulary that
could drift from the resource registry.

**Alternatives considered**: A hardcoded list of kind aliases (`"po"`, `"deploy"`, etc.) — rejected
for v1 as unnecessary scope; fuzzy-matching the token against the real `kind` string already lets
short prefixes like `"po"` match `"Pod"` without a separate alias table. Can be revisited later if
manual testing shows real friction.

## Decision: Pane/section search scope is `NAV_GROUPS`'s sections, not `ToolKind` dock tabs

**Decision**: "Tool pane" results (User Story 3) are drawn from `NAV_GROUPS`
(`ui/src/resources/nav.ts`), the sidebar's existing source of truth for navigable sections
(e.g. "Cluster Settings", "Port Forwards" if/when it is a `NAV_GROUPS` entry, etc.), selected via
the existing `navigate(section)` store action — the same primitive `NavTree.tsx` already uses.
Dock/tool tabs (`ToolKind`: `logs`, `events`, `portforwards`, `topology`, etc., in
`ui/src/layout/defaults.ts`) are not included as a separate palette category in this iteration.

**Rationale**: `NAV_GROUPS` is a single, already-labeled, already-navigable source of truth with
an existing dispatch action (`navigate`). `ToolKind` dock tabs have no existing label/icon map and
are a secondary, per-pane concept (which dock tab is visible in which pane slot) rather than a
single global "go to X" target — folding them in would require inventing new label data and a new
navigation primitive, which is unnecessary scope for a P3 story whose value is already delivered
by section-level jump (e.g. jumping to "Cluster Settings" already surfaces "Available Agents",
which lives inside that section per the Assumptions in spec.md).

**Alternatives considered**: Also indexing `ToolKind` values with a small hardcoded label map —
deferred; can be added later as a low-risk, additive change if operators want direct tab-level
jump, without blocking this feature's MVP scope.

## Decision: Global shortcut via a dedicated hook, not a new shortcut registry

**Decision**: A new small hook (colocated with `CommandPaletteHost.tsx`) attaches one
`document.addEventListener('keydown', ...)` listener on mount, checking
`(e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k'`, calling `e.preventDefault()` and
toggling/opening the palette's local `open` state.

**Rationale**: The codebase has exactly one precedent for a global key listener —
`Inspector.tsx`'s `useEscapeToClose` — and no general shortcut registry. Introducing a registry
now would be speculative generality for a single new shortcut; following the existing precedent
keeps the change minimal and consistent with current style.

**Alternatives considered**: A new generic "keyboard shortcut registry" module — rejected as
premature abstraction; nothing else in the app currently needs one, and the constitution favors
minimal, justified complexity.
