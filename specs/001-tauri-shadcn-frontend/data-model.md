# Data Model: Tauri + shadcn Frontend

**Plan**: [plan.md](./plan.md) · **Contracts**: [ipc](./contracts/ipc.md), [layout](./contracts/layout.md), [theme](./contracts/theme.md)

The backend's Kubernetes data model does not change. This document covers the entities the new frontend introduces or depends on.

## 1. Protocol (existing, Rust-owned, TS-generated)

| Entity | Source | Notes |
|---|---|---|
| `BackendCmd` | `src/protocol/v1/cmd.rs` | 137 variants plus 3 new ones (`LoadUiConfig`, `SaveUiConfig`, `SetMcpAutoStart`). Fire-and-forget. |
| `BackendEventEnvelope` | `src/protocol/v1/event.rs` | `{ version, seq, route, correlation, event }` |
| `Route` | `src/protocol/v1/route.rs` | `{ context?, section: SectionKey, scope: ScopeKind, namespace? }`. It keys list data in the store. |
| `Correlation` | `src/protocol/v1/ids.rs` | `{ request_id?, stream_id? }`. Used for staleness checks and stream routing. |
| `EventBatch` | `ui/src-tauri/src/bridge.rs` (new) | `{ envelopes, dropped }` |

Validation lives in Rust, and the frontend trusts the generated types. The only runtime check is on `version === 1`: a mismatched batch is dropped and a fatal banner is shown.

## 2. Persisted UI config (new, `~/.fathom/config.json → ui`)

```ts
type WidthClass = "phone" | "tablet" | "laptop" | "desktop" | "wide" | "ultra";

interface UiConfig {
  schema: 1;
  theme: {
    mode: "dark" | "light";          // dark = neon-noir, light = blueprint
    accent: "cyan" | "magenta";      // default "cyan"
    effects: "on" | "off";           // scanlines, glow, grain; default "on"
    glitch: boolean;                 // animated glitch on chrome; default false
  };
  density: { override?: "comfortable" | "compact" };  // optional, beats the class default
  layouts: Partial<Record<WidthClass, LayoutProfile>>;
  lastSelection?: { context: string; namespace?: string; section: SectionKey };
}

interface LayoutProfile {
  slots: SlotConfig[];               // left to right
  dock?: { open: boolean; heightPct: number; tabs: ToolKind[] };
}

interface SlotConfig {
  kind: "nav" | "list" | "inspector" | "tool";
  sizePct: number;                   // from react-resizable-panels
  pinnedRoute?: Route;               // list slots only
  tool?: ToolKind;                   // "overview" | "metrics" | "flux" | "logs" | "events" | "portforwards"
}
```

**Rules**:
- Unknown or invalid `ui` content, or `schema` ≠ 1, falls back to the defaults in `layout.md` without erasing the file. The bad blob is kept under `ui_invalid` for one save.
- `navigation.expanded_groups` stays where it is. The frontend reads and writes it through the same commands (Rust side: `UiConfigLoaded` carries both).
- A profile whose slot count exceeds `visibleSlots(class)` is truncated on load.
- Writes are debounced (500 ms) and happen only after user-initiated layout changes, never on automatic resizes.

## 3. Client store (Zustand, in memory)

```ts
interface Store {
  conn: { status: "starting" | "ready" | "backend-gone"; dropped: number };
  session: {
    contexts: ContextInfo[];
    activeContext?: string;
    namespaces: string[];
    activeNamespace: string | "*";
  };
  lists: Map<ListKey, ListState>;          // ListKey = `${context}|${section}|${scope}|${ns}`
  details: Map<DetailKey, DetailState>;    // DetailKey = `${context}|${kind}|${ns}|${name}`
  streams: Map<StreamId, StreamState>;     // logs / port-forward status
  metrics: { cluster?: ClusterMetrics; nodes: Map<string, NodeMetrics>; workloads: ... };
  flux: FluxState;
  panes: Map<PaneId, PaneState>;           // survives class changes (FR-013)
  layout: { widthClass: WidthClass; profile: LayoutProfile };
  selection: Map<PaneId, Set<RowKey>>;     // multi-select for bulk actions
  pending: Map<RequestId, PendingAction>;  // delete/scale/restart awaiting result
  ui: UiConfig;
}

interface ListState {
  rows: Row[]; latestRequest?: RequestId;
  status: "idle" | "loading" | "ready" | "error"; error?: string;
  updatedAt: number;
}
interface PaneState {
  id: PaneId; kind: PaneKind; route?: Route;
  search: string; sort?: { col: string; dir: "asc" | "desc" };
  scrollTop: number; focusedRow?: RowKey;
}
interface StreamState {
  kind: "logs" | "portforward"; lines: RingBuffer<LogLine>;   // capacity 10,000
  follow: boolean; status: "open" | "closed" | "error";
}
```

**The reducer** is `ingest(batch)`. It is one `set()` call per batch, and it applies these rules in order:

1. Drop an envelope if `route.context !== session.activeContext`, except for System events.
2. Drop a `ResourceList` envelope if `correlation.request_id < lists[key].latestRequest` (FR-016).
3. For list replacement, reuse the existing row objects when `(uid, resourceVersion)` is unchanged, so memoized rows don't re-render.
4. For stream events, append to the ring buffer. When it's over capacity, drop the oldest lines and increment `truncated`.
5. For action results, resolve `pending[request_id]` → toast (sonner), and clear the selection on success.

## 4. State transitions

**Connection**

```text
starting ──subscribe ok──▶ ready ──send_cmd BackendGone──▶ backend-gone (fatal banner, retry = app restart)
```

**List**

```text
idle ─refresh cmd─▶ loading ─list event─▶ ready ─context/ns change─▶ loading
                        └─error event─▶ error ─retry─▶ loading
```

**Destructive action (US3)**

```text
select rows → action → ConfirmDialog(shows count + names; type-to-confirm when count > 5)
  → send cmd, pending[rid] → result event → toast ✓/✗ → refresh list
```

The dialog is dismissed by Esc or Cancel, and no command is sent. There is no optimistic removal of rows.

**Pane under width change (US2)**

```text
visible in slot ──class shrinks──▶ stacked (phone) / tabbed (tablet) ──class grows──▶ visible in slot
```

`PaneState` is never discarded along this path. Only closing the pane explicitly removes it.

## 5. Resource descriptor (frontend config, one per kind)

```ts
interface ResourceDescriptor<R> {
  section: SectionKey; kind: string; namespaced: boolean;
  columns: Array<{
    id: string; header: string; priority: 1 | 2 | 3 | 4 | 5;
    minWidth: number; cell: (r: R) => ReactNode; sort?: (a: R, b: R) => number;
  }>;
  rowKey: (r: R) => RowKey;
  refresh: (route: Route) => BackendCmd;
  read?: (ref: ObjectRef) => BackendCmd[];        // detail + yaml + events
  detailSections: Array<DetailSection<R>>;
  actions: Array<ActionDescriptor<R>>;            // delete, scale, restart, logs, port-forward, …
  selectListEvent: (e: ResourceListEvent) => R[] | undefined;
}
```

Descriptors are pure data plus small render functions. A Vitest check asserts that every descriptor has a priority-1 name column and a status column, and that `minWidth` of its priority-1 columns fits in 358 px (a 390 px phone minus the gutters).
