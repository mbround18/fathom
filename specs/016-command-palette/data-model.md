# Phase 1 Data Model: Command Palette

No backend/protocol types are added (pure frontend feature). All shapes below are new
frontend-only TypeScript types living in `ui/src/resources/paletteIndex.ts`, derived from existing
store/resource types (`ObjectRef`, `ListState`, `ResourceDescriptor`, `NavGroup`) — they are not
persisted and not part of `ui/src/store/index.ts`'s `StoreData`.

## `PaletteQuery`

The parsed form of the operator's raw input string.

| Field | Type | Notes |
|---|---|---|
| `raw` | `string` | The unparsed input, kept for display/highlight purposes. |
| `kind` | `string \| null` | Text before a literal `:`, if present (e.g. `"pod"` from `"pod:my-app"`). `null` if no `:` in the input. |
| `name` | `string` | Text after the `:` if `kind` is set, otherwise the full raw input. Used for fuzzy matching against candidate labels. |

Parsing rule: split on the *first* `:` only (so a name containing `:` after the first still works
as free text); an empty `name` after a `kind:` prefix (e.g. `"pod:"`) yields all candidates of that
kind, unranked further than kind match.

## `PaletteResultCategory`

```ts
type PaletteResultCategory = 'resource' | 'namespace' | 'section'
```

Drives the visual grouping (`CommandGroup` headings) and which store action a selection triggers
(FR-006, FR-009–FR-011).

## `PaletteResult`

A single selectable entry, category-tagged so selection logic can dispatch correctly.

| Field | Type | Notes |
|---|---|---|
| `id` | `string` | Stable key for React/cmdk (`resource:${section}:${namespace}:${name}`, `namespace:${name}`, or `section:${key}`). |
| `category` | `PaletteResultCategory` | See above. |
| `label` | `string` | Primary display text (resource name, namespace name, or section label). |
| `sublabel` | `string \| undefined` | Secondary text: resource kind + namespace for resources (e.g. `"Pod · default"`), the nav group label for sections; omitted for namespaces. |
| `matchScore` | `number` | Higher = stronger match; used to sort within a category before capping (FR-007). Derived from `cmdk`'s own scoring (via `CommandItem`'s `value`) rather than a hand-rolled scorer — see research.md. |
| `payload` | `PaletteResourcePayload \| PaletteNamespacePayload \| PaletteSectionPayload` | Category-specific data needed to perform the navigation on select. |

```ts
interface PaletteResourcePayload {
  ref: ObjectRef // { section: SectionKey; kind: string; namespace: string | null; name: string }
}
interface PaletteNamespacePayload {
  namespace: string // never null here — "all namespaces" is not a palette result
}
interface PaletteSectionPayload {
  section: SectionKey
}
```

## Relationships / derivation

- `PaletteResult[category=resource]` is derived from `store.lists` (filtered to the active
  context) × `descriptorFor(section)` → `descriptor.ref(row)` for each row, per research.md's
  "no new index" decision. Not persisted; recomputed from live store state each time the palette
  is queried (or memoized per open-session if profiling shows it matters — no requirement forces
  either choice).
- `PaletteResult[category=namespace]` is derived from the store's existing loaded namespace list
  (the `namespaces` section's rows, if loaded) — same "already loaded" constraint as resources
  (FR-016: namespace search must still work even if resource lists aren't loaded, since the
  namespace list is typically loaded early/always for the namespace filter UI).
- `PaletteResult[category=section]` is derived statically from `NAV_GROUPS`
  (`ui/src/resources/nav.ts`) — always available regardless of connection state, satisfying the
  "pane search still works with no resources loaded" edge case.

## Validation / invariants

- A `PaletteResult` list presented to the operator MUST be capped in total count (FR-007) —
  enforced in `paletteIndex.ts`'s ranking function, not in the UI component, so it's covered by
  unit tests.
- Resource results MUST NOT be included for kinds outside the active context's loaded lists
  (FR-012) — enforced by only reading `lists` entries whose `ListKey` context segment matches
  `store.session`'s active context.
- No `PaletteResult` may reference resource content (labels/annotations/spec/status) per FR-013 —
  the index only ever reads `descriptor.ref(row)` (kind/namespace/name) and the row's own display
  name, never full row/detail payloads.
