/**
 * App state (data-model.md §3). One Zustand store; components subscribe with narrow selectors.
 *
 * Only the shell's slice is implemented here. Transports, event ingest, and resource caches
 * (T028–T037) extend this store later.
 */
import { create } from 'zustand'

import { resolveProfile } from '@/layout/defaults'
import type { WidthClass } from '@/layout/width-class'
import type {
  ContextInfo,
  LayoutProfile,
  Route,
  SectionKey,
  SlotConfig,
  ThemePrefs,
  ToolKind,
  UiConfig,
} from '@/protocol/placeholders'

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

// TODO(protocol): replace with generated types (ConnectionState from contracts/ipc.md)
export type ConnectionStatus = 'connecting' | 'connected' | 'reconnecting' | 'disconnected'

export interface Notice {
  level: 'info' | 'warn' | 'error'
  message: string
  at: number
}

export interface ConnState {
  status: ConnectionStatus
  /** Events dropped by the bridge because the UI fell behind (backpressure). */
  dropped: number
  lastNotice?: Notice
}

export interface SessionState {
  contexts: ContextInfo[]
  activeContext?: string
  namespaces: string[]
  /** `null` means all namespaces. */
  activeNamespace: string | null
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

export interface StoreState {
  conn: ConnState
  session: SessionState
  layout: LayoutState
  panes: Record<PaneId, PaneState>
  ui: UiConfig

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
  selectContext: (name: string) => void
  selectNamespace: (ns: string | null) => void
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
  const slotPanes = profile.slots.map((s) => claim(s.kind, s.kind === 'tool' ? s.tool : undefined))
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

const DEFAULT_UI: UiConfig = { schema: 1, theme: DEFAULT_THEME, density: {}, layouts: {} }

function initialState(): Pick<StoreState, 'conn' | 'session' | 'layout' | 'panes' | 'ui'> {
  const profile = resolveProfile('phone')
  const { slotPanes, dockPanes, panes } = assignPanes(profile, {})
  return {
    conn: { status: 'connecting', dropped: 0 },
    session: { contexts: [], namespaces: [], activeNamespace: null },
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
    ui: DEFAULT_UI,
  }
}

export const useStore = create<StoreState>()((set) => ({
  ...initialState(),

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

  navigate: (section) =>
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
    }),

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

  selectContext: (name) => set((s) => ({ session: { ...s.session, activeContext: name } })),

  selectNamespace: (ns) => set((s) => ({ session: { ...s.session, activeNamespace: ns } })),
}))
