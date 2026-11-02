# Data model: Shared UI kit

There is no stored data. These are the UI-level shapes the kit and the apps exchange.

## Kit piece

| Field | Meaning |
|---|---|
| `id` | kebab-case id, for example `empty-state`. Names the harness fixture (`harness/pieces/<id>.tsx`) and the spec (`tests/e2e/<id>.spec.ts`). |
| `exports` | the names exported from `@companion/ui/kit` (contracts/kit-api.md) |
| `states` | the states its fixture renders, for example `guided`, `plain`, `filtered`, `restricted` |
| `markers` | the `data-` attributes it keeps from Keel (FR-003) |
| `appContent` | the props and provider fields it takes from the app |

Rules:

- Each piece has a fixture, a unit test and an isolated spec (FR-016, FR-017).
- The piece's spec runs every state through `expectAccessible`.
- The kit never imports from `apps/*`, `ui/*` or `*/protocol/*`.

## App content (`KitContext`)

```ts
interface KitContext {
  /** In-app link component. Default: the kit's own `Link`. */
  Link?: ComponentType<Omit<ComponentProps<'a'>, 'href'> & { to: string }>
  /** Builds a guide's route. Absent: GuideLink renders nothing. */
  guideHref?: (ref: GuideRef) => string
  /** Tells whether a Markdown link target (`slug.md`) is one of the app's guides. */
  isGuide?: (slug: string) => boolean
  /** The app's glossary. Absent: TermTip renders nothing. */
  glossary?: Readonly<Record<string, GlossaryEntry>>
  /** The bundled URL of a guide image (`img/<name>`). Absent or `undefined`: the image shows as a link. */
  guideImage?: (name: string) => string | undefined
}
interface GuideRef { guide: string; anchor?: string }
interface GlossaryEntry { term: string; text: string; guide?: GuideRef }
```

## Refusal

The type below is moved verbatim from Keel's `app/load.ts`.

```ts
type Refusal =
  | { kind: 'denied'; action: string; reason: string }
  | { kind: 'error'; code: string; message: string }
```

- `isTransportFailure(r)` is true exactly when `r.kind === 'error' && r.code === 'offline'`.
- The app owns the `OFFLINE` value, with its own wording, and maps its protocol to this shape.
- `RefusalText` renders it as follows:

| Refusal | Output |
|---|---|
| `denied` | `Denied: <action> (<reason>)` |
| transport failure | the message alone |
| any other `error` | `<message> (<code>)` |

## List load state

```ts
type ListLoad<T> =
  | { status: 'loading' }
  | { status: 'loaded'; value: T }
  | { status: 'failed'; refusal: Refusal }
```

- This is optional. Keel's stores keep their own fields, and `LoadState` takes
  `loading` / `refusal` / `onRetry` props as it does today.
- New Depths stores may use `ListLoad<T>`.

## Navigation

```ts
interface NavItem {
  path: string
  label: string
  icon: Icon
  badge?: { count: number; label: string }
  dot?: { label: string }
}
interface NavGroup { id: string; label: string; items: readonly NavItem[] }
```

- `badge` renders only when `count > 0`. `label` is the screen-reader-only text, for example
  "3 waiting for approval".
- `dot` is the update dot (`data-update-dot`).

## Table sort, selection and actions (DataTable and DataGrid)

```ts
interface SortState { key: string; dir: 'asc' | 'desc' }
interface TableSort { value: SortState | null; onChange: (s: SortState | null) => void }
interface TableSelection { value: ReadonlySet<string>; onChange: (s: Set<string>) => void }
interface RowAction { label: string; onSelect: () => void; destructive?: boolean; disabled?: boolean }
type RowActions<T> = (row: T) => readonly RowAction[]
```

- A column is sortable only when it has `sortable: true`.
- Clicking a column header cycles its sort through ascending, descending, then none.
- Rows are keyed by `rowKey(row): string`. `rowLabel(row)` names a row in "Select …" and
  "Actions for …"; it defaults to `rowKey`.
- The app orders and filters `rows`; neither component sorts.

### DataGrid only

```ts
interface GridColumn<T> {
  id: string; header: string
  cell: (row: T) => ReactNode; text: (row: T) => string   // text: title and long-press peek
  priority: number; minWidth: number; grow?: number       // column budget
  align?: 'start' | 'end'; sortable?: boolean; className?: string
}
interface GridCursor { value: string | null | undefined; onChange: (key: string) => void }
interface RowMenuRequest { key: string; x: number; y: number; peek?: string }
```

- `cursor` is the keyboard cursor (internal when omitted); `openKey` is the open row
  (`aria-current`); `selection` checks rows (`aria-selected`).
- The row menu is the kit's (`rowActions`) or the app's (`onRowMenu(req)`), never both.
- `onKeyDown(e, cursor)` runs before the grid's keys; `preventDefault` there stops them.

## Allow-listed copy (`packages/ui/kit-allow.json`)

```json
[{ "file": "ui/src/components/Foo.tsx", "name": "EmptyState", "reason": "…" }]
```

- `reason` is required and must not be empty.
- An entry that no longer matches anything fails the check, so the list can't go stale.
