/**
 * App state (data-model.md §3). One Zustand store; components subscribe with narrow selectors.
 *
 * Backend data arrives only through `ingest(batch)` (./ingest.ts), which applies one `set()`
 * per batch. Actions that need the backend call `send`, which `useBackend` wires to the
 * transport; until then it is a no-op.
 */
import { create } from 'zustand'

import { resolveProfile } from '@/layout/defaults'
import type { WidthClass } from '@/layout/width-class'
import { namespaceFilterCmd, refreshCmd, SECTION, selectCmd } from '@/protocol/commands'
import type {
  BackendCmd,
  DiskUsage,
  FluxOverview,
  KubernetesEvent,
  LayoutProfile,
  MetricsStackScanResult,
  ResourceListEvent,
  ResourceReadEvent,
  Route,
  SectionKey,
  SlotConfig,
  ThemePrefs,
  ToolKind,
  TunnelStatus,
  UiConfig,
} from '@/protocol/types'

import { detailKey, listKey, type DetailKey, type ListKey } from './keys'
import type { RingBuffer } from './ring-buffer'

export { detailKey, listKey, listKeyOf, type DetailKey, type ListKey } from './keys'

// ------------------------------------------------------------------ UI-owned types

export type PaneId = string
export type PaneKind = SlotConfig['kind']

/** Per-pane view state. Lives here, not in components, so re-layout never loses it (FR-007). */
export interface PaneState {
  id: PaneId
  kind: PaneKind
  tool?: ToolKind
  route?: Route
  search: string
  selection?: string
  scrollTop: number
}

/** data-model.md §4: starting → ready → backend-gone (fatal; retry = app restart). */
export type ConnectionStatus = 'starting' | 'ready' | 'backend-gone'

export interface Notice {
  level: 'info' | 'warn' | 'error'
  message: string
  at: number
}

export interface ConnState {
  status: ConnectionStatus
  /** Envelopes the bridge discarded because the UI fell behind (backpressure), cumulative. */
  dropped: number
  /** Why the connection is unusable (backend gone, protocol mismatch). Shown verbatim. */
  fatal?: string
  lastNotice?: Notice
}

export interface SessionState {
  /** Kubeconfig context names. Empty with `contextsLoaded` means no kubeconfig. */
  contexts: string[]
  contextsLoaded: boolean
  activeContext?: string
  namespaces: string[]
  /** `null` means all namespaces. */
  activeNamespace: string | null
  /** The backend's namespace filter as last reported (`[]` = all). */
  namespaceFilter: string[]
}

type RowOf<T> = T extends readonly (infer R)[] ? R : T extends { events: (infer E)[] } ? E : never
/** Any row a `ResourceList` event can carry. */
export type ListRow = RowOf<ResourceListEvent['data']>

export interface ListState {
  /** Which `ResourceListEvent` variant filled this list. */
  eventType?: ResourceListEvent['type']
  rows: readonly ListRow[]
  /** For events: the backend's total before truncation. */
  total?: number
  latestRequest?: number
  status: 'idle' | 'loading' | 'ready' | 'error'
  error?: string
  updatedAt: number
}

export interface DetailState {
  /** Object kind derived from the event (`PodDetailsUpdated` → `Pod`, `PodMetricsUpdated` → `PodMetrics`). */
  kind: string
  eventType: ResourceReadEvent['type']
  data: ResourceReadEvent['data']
  latestRequest?: number
  updatedAt: number
}

export interface LogStreamState {
  kind: 'logs'
  lines: RingBuffer<string>
  /** Bumped on every append; the buffer itself is mutated in place. */
  rev: number
  follow: boolean
  status: 'open' | 'closed' | 'error'
  reason?: string
}

export interface TunnelState {
  kind: 'portforward'
  status: TunnelStatus
  localEndpoint: string | null
}

export interface PendingAction {
  /** Shown in the result toast, e.g. "Delete 3 pods". */
  label: string
  /** Pane whose selection is cleared when the action succeeds. */
  paneId?: PaneId
  sentAt: number
}

export interface MetricsState {
  diskUsage: DiskUsage[]
  stackScan?: MetricsStackScanResult
}

export interface IntegrationsState {
  mcp?: { running: boolean; message: string }
  prometheusTest?: { success: boolean; message: string }
  grafanaTest?: { success: boolean; message: string }
  portAvailability: Record<number, boolean>
}

export interface PrefsState {
  /** True once `UiConfigLoaded` has arrived. */
  loaded: boolean
  expandedGroups: string[]
  mcpAutoStart: boolean
}

export interface LayoutState {
  widthClass: WidthClass
  ultraPlus: boolean
  profile: LayoutProfile
  /** Pane shown in each visible slot, index-aligned with `profile.slots`. */
  slotPanes: PaneId[]
  /** Pane shown in each dock tab, index-aligned with `profile.dock.tabs`. */
  dockPanes: PaneId[]
  /**
   * Phone navigation stack; the last entry is on screen. On tablet the root entry picks the
   * active tab (a tool pane, or anything else for the list/inspector split).
   */
  stack: PaneId[]
  /** Navigator drawer (phone/tablet, where nav has no slot). */
  navOpen: boolean
}

/** The data part of the store (everything `ingest` may replace). */
export interface StoreData {
  conn: ConnState
  session: SessionState
  lists: Record<ListKey, ListState>
  details: Record<DetailKey, DetailState>
  /** Log streams by `StreamId`. */
  streams: Record<number, LogStreamState>
  /** Port-forward tunnels by tunnel id. */
  tunnels: Record<string, TunnelState>
  metrics: MetricsState
  flux: { overview?: FluxOverview }
  integrations: IntegrationsState
  layout: LayoutState
  panes: Record<PaneId, PaneState>
  /** Multi-select for bulk actions, by pane. */
  selection: Record<PaneId, string[]>
  /** Actions awaiting a result, by request id. */
  pending: Record<number, PendingAction>
  ui: UiConfig
  prefs: PrefsState
}

export interface StoreState extends StoreData {
  /** Fire-and-forget command sink; `useBackend` installs the transport-backed one. */
  send: (cmd: BackendCmd) => void

  setWidthClass: (cls: WidthClass, ultraPlus?: boolean) => void
  /** Persist a resized/rearranged profile for the current class. */
  setProfile: (profile: LayoutProfile) => void
  setTheme: (patch: Partial<ThemePrefs>) => void
  updatePane: (id: PaneId, patch: Partial<Omit<PaneState, 'id' | 'kind'>>) => void
  pushPane: (id: PaneId) => void
  /** Phone: make the pane for `kind`/`tool` the root of the stack, creating it if needed. */
  openRoot: (kind: PaneKind, tool?: ToolKind) => void
  /** Show `section` in the first visible list pane (the phone's list pane when narrow). */
  navigate: (section: SectionKey) => void
  popPane: () => void
  setNavOpen: (open: boolean) => void
  setConnection: (patch: Partial<ConnState>) => void
  notify: (level: Notice['level'], message: string) => void
  /** Switch kubeconfig context: sends `SetContext`; results for other contexts are then dropped. */
  selectContext: (name: string) => void
  /** Sends `SetNamespaceFilter` (`null` = all namespaces). */
  selectNamespace: (ns: string | null) => void
  /** Sends the section's `Refresh*` command and marks its list loading. */
  refreshSection: (section: SectionKey) => void
  /** Sends `Select<Kind>` to read one object's details (and its `raw_json`/YAML). */
  inspect: (kind: string, namespace: string, name: string) => void
  /** Sends `RefreshEvents` (cluster events; the inspector filters by involved object). */
  refreshEvents: () => void
}

/** Lists the backend refreshes after a context or namespace change (src/backend.rs). */
const CONTEXT_LISTS: readonly SectionKey[] = [SECTION.pods, SECTION.deployments]

function markLoading(
  lists: Record<ListKey, ListState>,
  context: string | undefined,
  sections: readonly SectionKey[],
): Record<ListKey, ListState> {
  if (context === undefined) return lists
  const next = { ...lists }
  for (const section of sections) {
    // The backend reports namespaced lists with namespace `null` and filters server-side.
    const key = listKey(context, section, 'Namespaced', null)
    const prev = next[key]
    next[key] = prev
      ? { ...prev, status: 'loading', error: undefined }
      : { rows: [], status: 'loading', updatedAt: 0 }
  }
  return next
}

// ------------------------------------------------------------------ pane assignment

function newPane(id: PaneId, kind: PaneKind, tool?: ToolKind): PaneState {
  return { id, kind, ...(tool ? { tool } : {}), search: '', scrollTop: 0 }
}

function nextId(panes: Record<PaneId, PaneState>, prefix: string): PaneId {
  let n = 1
  while (`${prefix}-${n}` in panes) n++
  return `${prefix}-${n}`
}

/**
 * Map each slot to a pane: reuse the first unused pane of the same kind (and tool), else create
 * one. Panes that no longer have a slot are kept, so widening again restores their state.
 */
export function assignPanes(
  profile: LayoutProfile,
  existing: Record<PaneId, PaneState>,
): { slotPanes: PaneId[]; dockPanes: PaneId[]; panes: Record<PaneId, PaneState> } {
  const panes = { ...existing }
  const used = new Set<PaneId>()
  const claim = (kind: PaneKind, tool?: ToolKind, prefix: string = tool ?? kind): PaneId => {
    const found = Object.values(panes).find(
      (p) => !used.has(p.id) && p.kind === kind && p.tool === tool && p.id.startsWith(prefix),
    )
    const id = found?.id ?? nextId(panes, prefix)
    if (!found) panes[id] = newPane(id, kind, tool)
    used.add(id)
    return id
  }
  const slotPanes = profile.slots.map((s) =>
    claim(s.kind, s.kind === 'tool' ? (s.tool ?? undefined) : undefined),
  )
  const dockPanes = (profile.dock?.tabs ?? []).map((tool) => claim('tool', tool, `dock-${tool}`))
  return { slotPanes, dockPanes, panes }
}

// ------------------------------------------------------------------ defaults

export const DEFAULT_THEME: ThemePrefs = {
  mode: 'dark',
  accent: 'cyan',
  effects: 'on',
  glitch: false,
}

export const DEFAULT_UI: UiConfig = { schema: 1, theme: DEFAULT_THEME, density: {}, layouts: {} }

/** A fresh copy of the store's data (also used by tests to reset between cases). */
export function initialData(): StoreData {
  const profile = resolveProfile('phone')
  const { slotPanes, dockPanes, panes } = assignPanes(profile, {})
  return {
    conn: { status: 'starting', dropped: 0 },
    session: {
      contexts: [],
      contextsLoaded: false,
      namespaces: [],
      activeNamespace: null,
      namespaceFilter: [],
    },
    lists: {},
    details: {},
    streams: {},
    tunnels: {},
    metrics: { diskUsage: [] },
    flux: {},
    integrations: { portAvailability: {} },
    layout: {
      widthClass: 'phone',
      ultraPlus: false,
      profile,
      slotPanes,
      dockPanes,
      stack: slotPanes.slice(0, 1),
      navOpen: false,
    },
    panes,
    selection: {},
    pending: {},
    ui: DEFAULT_UI,
    prefs: { loaded: false, expandedGroups: [], mcpAutoStart: false },
  }
}

export const useStore = create<StoreState>()((set, get) => ({
  ...initialData(),

  send: () => undefined,

  setWidthClass: (cls, ultraPlus = false) =>
    set((s) => {
      if (s.layout.widthClass === cls && s.layout.ultraPlus === ultraPlus) return s
      const profile = resolveProfile(cls, s.ui.layouts[cls], ultraPlus)
      const { slotPanes, dockPanes, panes } = assignPanes(profile, s.panes)
      const stack = s.layout.stack.filter((id) => id in panes)
      return {
        panes,
        layout: {
          widthClass: cls,
          ultraPlus,
          profile,
          slotPanes,
          dockPanes,
          stack: stack.length > 0 ? stack : slotPanes.slice(0, 1),
          navOpen: false,
        },
      }
    }),

  setProfile: (saved) =>
    set((s) => {
      const cls = s.layout.widthClass
      const profile = resolveProfile(cls, saved, s.layout.ultraPlus)
      const { slotPanes, dockPanes, panes } = assignPanes(profile, s.panes)
      return {
        panes,
        layout: { ...s.layout, profile, slotPanes, dockPanes },
        ui: { ...s.ui, layouts: { ...s.ui.layouts, [cls]: profile } },
      }
    }),

  setTheme: (patch) => set((s) => ({ ui: { ...s.ui, theme: { ...s.ui.theme, ...patch } } })),

  updatePane: (id, patch) =>
    set((s) => {
      const pane = s.panes[id]
      return pane ? { panes: { ...s.panes, [id]: { ...pane, ...patch } } } : s
    }),

  pushPane: (id) =>
    set((s) =>
      s.layout.stack.at(-1) === id
        ? s
        : { layout: { ...s.layout, stack: [...s.layout.stack, id] } },
    ),

  openRoot: (kind, tool) =>
    set((s) => {
      const prefix = tool ?? kind
      const found = Object.values(s.panes).find(
        (p) => p.kind === kind && p.tool === tool && p.id.startsWith(`${prefix}-`),
      )
      const id = found?.id ?? nextId(s.panes, prefix)
      return {
        panes: found ? s.panes : { ...s.panes, [id]: newPane(id, kind, tool) },
        layout: { ...s.layout, stack: [id] },
      }
    }),

  navigate: (section) => {
    set((s) => {
      const listId =
        s.layout.slotPanes.find((id) => s.panes[id]?.kind === 'list') ??
        Object.values(s.panes).find((p) => p.kind === 'list')?.id
      const pane = listId ? s.panes[listId] : undefined
      if (!pane) return s
      const route: Route = {
        context: s.session.activeContext ?? null,
        section,
        scope: 'Namespaced',
        namespace: s.session.activeNamespace,
      }
      return {
        panes: { ...s.panes, [pane.id]: { ...pane, route, selection: undefined, scrollTop: 0 } },
        layout: {
          ...s.layout,
          navOpen: false,
          stack:
            s.layout.widthClass === 'phone' || s.layout.widthClass === 'tablet'
              ? [pane.id]
              : s.layout.stack,
        },
      }
    })
    get().refreshSection(section)
  },

  popPane: () =>
    set((s) =>
      s.layout.stack.length <= 1
        ? s
        : { layout: { ...s.layout, stack: s.layout.stack.slice(0, -1) } },
    ),

  setNavOpen: (open) => set((s) => ({ layout: { ...s.layout, navOpen: open } })),

  setConnection: (patch) => set((s) => ({ conn: { ...s.conn, ...patch } })),

  notify: (level, message) =>
    set((s) => ({ conn: { ...s.conn, lastNotice: { level, message, at: Date.now() } } })),

  selectContext: (name) => {
    const s = get()
    if (s.session.activeContext === name) return
    set({
      session: {
        ...s.session,
        activeContext: name,
        activeNamespace: null,
        namespaceFilter: [],
        namespaces: [],
      },
      lists: markLoading(s.lists, name, CONTEXT_LISTS),
    })
    s.send({ type: 'SetContext', data: { context: name } })
  },

  selectNamespace: (ns) => {
    const s = get()
    set({
      session: { ...s.session, activeNamespace: ns },
      lists: markLoading(s.lists, s.session.activeContext, CONTEXT_LISTS),
    })
    s.send(namespaceFilterCmd(ns))
  },

  refreshSection: (section) => {
    const cmd = refreshCmd(section)
    if (!cmd) return
    const s = get()
    set({ lists: markLoading(s.lists, s.session.activeContext, [section]) })
    s.send(cmd)
  },

  inspect: (kind, namespace, name) => {
    const cmd = selectCmd(kind, namespace, name)
    if (cmd) get().send(cmd)
  },

  refreshEvents: () => get().send({ type: 'RefreshEvents' }),
}))

/** Detail cache lookup for the active context. */
export function selectDetail(
  s: StoreData,
  kind: string,
  namespace: string | null,
  name: string,
): DetailState | undefined {
  return s.details[detailKey(s.session.activeContext, kind, namespace, name)]
}

/** Cluster events whose involved object is `kind`/`namespace`/`name` (inspector Events tab). */
export function eventsFor(
  s: StoreData,
  kind: string,
  namespace: string,
  name: string,
): KubernetesEvent[] {
  const list = s.lists[listKey(null, SECTION.events, 'Global', null)]
  return (list?.rows ?? []).filter(
    (e): e is KubernetesEvent =>
      typeof e === 'object' &&
      'involved_object_kind' in e &&
      e.involved_object_kind === kind &&
      e.involved_object_name === name &&
      e.namespace === namespace,
  )
}
