/**
 * The tab layout (data-model "Tab", FR-024, FR-025): which tabs are open, how they sit in panes,
 * and the panes' sizes. Pure data and functions, so it can be tested without a browser.
 *
 * Persisted per person and browser in `localStorage` under `depths:layout:<subject>`. Storage is a
 * convenience: every read and write is wrapped, and a page without storage works with an empty
 * layout. Restoring is defensive, since the stored text may come from an older build or be hand
 * edited: anything not understood is dropped rather than trusted.
 */
import type { LogTarget, Shell, StatsTarget } from '@/protocol/generated'

export type TabKind = 'logs' | 'shell' | 'stats'

export type TabSpec =
  | { id: string; kind: 'logs'; target: LogTarget }
  | {
      id: string
      kind: 'shell'
      container: string
      shell?: Shell
      /**
       * The session ended because the person was signed out: after signing in, offer a new
       * session rather than starting one unasked.
       */
      offerNew?: boolean
    }
  | { id: string; kind: 'stats'; target: StatsTarget }

export interface Pane {
  id: string
  tabs: string[]
  active: string | null
}

export interface Layout {
  tabs: TabSpec[]
  panes: Pane[]
  /** Panel id (`pane-…`, `page`, `dock`) to size in percent, as `react-resizable-panels` reports. */
  sizes: Record<string, number>
  /** The tab most recently focused, across panes; the one shown at narrow widths. */
  focused: string | null
  /** Whether the tab dock is shown. */
  open: boolean
}

/** Side-by-side panes at wide widths. */
export const MAX_PANES = 3
/** Each tab is at most one stream; the server allows 32 per socket. */
export const MAX_TABS = 16
const LAYOUT_VERSION = 1

export const EMPTY_LAYOUT: Layout = { tabs: [], panes: [], sizes: {}, focused: null, open: false }

/** The storage key for one person. */
export function layoutKey(subject: string): string {
  return `depths:layout:${subject}`
}

export function serialise(layout: Layout): string {
  return JSON.stringify({ v: LAYOUT_VERSION, ...layout })
}

// --- Restore -------------------------------------------------------------------------------------

type Json = Record<string, unknown>

const isObj = (v: unknown): v is Json => typeof v === 'object' && v !== null && !Array.isArray(v)
const isStr = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && v.length <= 255
/** What docker accepts as a container ref, and what a stack or service name may look like. */
const REF = /^[a-zA-Z0-9][a-zA-Z0-9._-]*$/
const isRef = (v: unknown): v is string => isStr(v) && REF.test(v)
const SHELLS: readonly Shell[] = ['sh', 'bash', 'ash']

function restoreTab(raw: unknown): TabSpec | null {
  if (!isObj(raw) || !isStr(raw.id)) return null
  const id = raw.id
  switch (raw.kind) {
    case 'logs': {
      const t = raw.target
      if (!isObj(t)) return null
      if (t.kind === 'stack' && isRef(t.stack)) {
        if (t.service === undefined) return { id, kind: 'logs', target: { kind: 'stack', stack: t.stack } }
        if (isRef(t.service)) return { id, kind: 'logs', target: { kind: 'stack', stack: t.stack, service: t.service } }
        return null
      }
      if (t.kind === 'container' && isRef(t.container)) {
        return { id, kind: 'logs', target: { kind: 'container', container: t.container } }
      }
      return null
    }
    case 'shell': {
      if (!isRef(raw.container)) return null
      const tab: TabSpec = { id, kind: 'shell', container: raw.container }
      if (SHELLS.includes(raw.shell as Shell)) tab.shell = raw.shell as Shell
      if (raw.offerNew === true) tab.offerNew = true
      return tab
    }
    case 'stats': {
      const t = raw.target
      if (!isObj(t)) return null
      if (t.kind === 'stack' && isRef(t.stack)) return { id, kind: 'stats', target: { kind: 'stack', stack: t.stack } }
      if (t.kind === 'container' && isRef(t.container)) {
        return { id, kind: 'stats', target: { kind: 'container', container: t.container } }
      }
      return null
    }
    default:
      return null
  }
}

/**
 * A layout from stored text. Unknown or malformed tabs are dropped, panes keep only tabs that
 * exist, empty panes go, tabs no pane holds join the first, and every pointer is made valid.
 */
export function restore(text: string | null): Layout {
  if (text === null) return EMPTY_LAYOUT
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch {
    return EMPTY_LAYOUT
  }
  if (!isObj(raw) || raw.v !== LAYOUT_VERSION) return EMPTY_LAYOUT

  const tabs: TabSpec[] = []
  const seen = new Set<string>()
  for (const t of Array.isArray(raw.tabs) ? raw.tabs : []) {
    const tab = restoreTab(t)
    if (tab && !seen.has(tab.id) && tabs.length < MAX_TABS) {
      seen.add(tab.id)
      tabs.push(tab)
    }
  }

  const placed = new Set<string>()
  const panes: Pane[] = []
  for (const p of Array.isArray(raw.panes) ? raw.panes : []) {
    if (!isObj(p) || !isStr(p.id) || !Array.isArray(p.tabs) || panes.length >= MAX_PANES) continue
    const ids: string[] = []
    for (const id of p.tabs) {
      if (isStr(id) && seen.has(id) && !placed.has(id)) {
        placed.add(id)
        ids.push(id)
      }
    }
    if (ids.length === 0) continue
    const active = isStr(p.active) && ids.includes(p.active) ? p.active : (ids[0] ?? null)
    panes.push({ id: p.id, tabs: ids, active })
  }
  const loose = tabs.map((t) => t.id).filter((id) => !placed.has(id))
  if (loose.length > 0) {
    const first = panes[0]
    if (first) first.tabs.push(...loose)
    else panes.push({ id: 'pane-1', tabs: loose, active: loose[0] ?? null })
  }

  const sizes: Record<string, number> = {}
  if (isObj(raw.sizes)) {
    for (const [k, v] of Object.entries(raw.sizes)) {
      if (isStr(k) && typeof v === 'number' && Number.isFinite(v) && v > 0 && v <= 100) sizes[k] = v
    }
  }
  const focused = isStr(raw.focused) && seen.has(raw.focused) ? raw.focused : (panes[0]?.active ?? null)
  return { tabs, panes, sizes, focused, open: raw.open === true && tabs.length > 0 }
}

// --- Storage -------------------------------------------------------------------------------------

/** The part of `Storage` the layout needs; `null` when the browser offers none. */
export type StorageLike = Pick<Storage, 'getItem' | 'setItem'>

export function browserStorage(): StorageLike | null {
  try {
    return globalThis.localStorage
  } catch {
    return null
  }
}

export function loadLayout(subject: string, storage: StorageLike | null = browserStorage()): Layout {
  try {
    return restore(storage?.getItem(layoutKey(subject)) ?? null)
  } catch {
    return EMPTY_LAYOUT
  }
}

/** Saves; `false` when storage refused (private mode, quota). The page carries on either way. */
export function saveLayout(subject: string, layout: Layout, storage: StorageLike | null = browserStorage()): boolean {
  try {
    if (!storage) return false
    storage.setItem(layoutKey(subject), serialise(layout))
    return true
  } catch {
    return false
  }
}

// --- Edits ---------------------------------------------------------------------------------------

/** Two tabs that would show the same thing. Shells are never deduplicated: each is a session. */
export function sameTarget(a: TabSpec, b: TabSpec): boolean {
  if (a.kind === 'logs' && b.kind === 'logs') return JSON.stringify(a.target) === JSON.stringify(b.target)
  if (a.kind === 'stats' && b.kind === 'stats') return JSON.stringify(a.target) === JSON.stringify(b.target)
  return false
}

export function paneOf(layout: Layout, tabId: string): Pane | undefined {
  return layout.panes.find((p) => p.tabs.includes(tabId))
}

function newPaneId(layout: Layout): string {
  for (let i = 1; ; i++) {
    const id = `pane-${String(i)}`
    if (!layout.panes.some((p) => p.id === id)) return id
  }
}

/** Adds `tab` to the focused pane (or the first), focused; an existing same-target tab is focused instead. */
export function addTab(layout: Layout, tab: TabSpec): Layout {
  const existing = layout.tabs.find((t) => sameTarget(t, tab))
  if (existing) return focusTab(layout, existing.id)
  if (layout.tabs.length >= MAX_TABS) return layout
  const into = (layout.focused ? paneOf(layout, layout.focused) : undefined) ?? layout.panes[0]
  const panes = into
    ? layout.panes.map((p) => (p === into ? { ...p, tabs: [...p.tabs, tab.id], active: tab.id } : p))
    : [{ id: newPaneId(layout), tabs: [tab.id], active: tab.id }]
  return { ...layout, tabs: [...layout.tabs, tab], panes, focused: tab.id, open: true }
}

export function focusTab(layout: Layout, tabId: string): Layout {
  const pane = paneOf(layout, tabId)
  if (!pane) return layout
  return {
    ...layout,
    panes: layout.panes.map((p) => (p === pane ? { ...p, active: tabId } : p)),
    focused: tabId,
    open: true,
  }
}

function prune(layout: Layout): Layout {
  const panes = layout.panes.filter((p) => p.tabs.length > 0)
  const focused =
    layout.focused && layout.tabs.some((t) => t.id === layout.focused) ? layout.focused : (panes[0]?.active ?? null)
  return { ...layout, panes, focused, open: layout.open && layout.tabs.length > 0 }
}

export function closeTab(layout: Layout, tabId: string): Layout {
  const panes = layout.panes.map((p) => {
    if (!p.tabs.includes(tabId)) return p
    const i = p.tabs.indexOf(tabId)
    const tabs = p.tabs.filter((id) => id !== tabId)
    const active = p.active === tabId ? (tabs[Math.min(i, tabs.length - 1)] ?? null) : p.active
    return { ...p, tabs, active }
  })
  return prune({ ...layout, tabs: layout.tabs.filter((t) => t.id !== tabId), panes })
}

/** Moves a tab to pane `to`, or to a new pane at the end (`'new'`, while fewer than `MAX_PANES`). */
export function moveTab(layout: Layout, tabId: string, to: number | 'new'): Layout {
  const from = paneOf(layout, tabId)
  if (!from) return layout
  if (to === 'new' && (layout.panes.length >= MAX_PANES || from.tabs.length === 1)) return layout
  const target = to === 'new' ? undefined : layout.panes[to]
  if (to !== 'new' && (!target || target === from)) return layout

  let panes = layout.panes.map((p) => {
    if (p === from) {
      const tabs = p.tabs.filter((id) => id !== tabId)
      return { ...p, tabs, active: p.active === tabId ? (tabs[0] ?? null) : p.active }
    }
    if (p === target) return { ...p, tabs: [...p.tabs, tabId], active: tabId }
    return p
  })
  if (to === 'new') panes = [...panes, { id: newPaneId(layout), tabs: [tabId], active: tabId }]
  return prune({ ...layout, panes, focused: tabId })
}

/** Moves a tab one place left or right within its pane (the strip's order is the order restored). */
export function reorderTab(layout: Layout, tabId: string, delta: -1 | 1): Layout {
  const pane = paneOf(layout, tabId)
  if (!pane) return layout
  const i = pane.tabs.indexOf(tabId)
  const j = i + delta
  if (j < 0 || j >= pane.tabs.length) return layout
  const tabs = [...pane.tabs]
  ;[tabs[i], tabs[j]] = [tabs[j] ?? tabId, tabs[i] ?? tabId]
  return { ...layout, panes: layout.panes.map((p) => (p === pane ? { ...p, tabs } : p)) }
}

export function updateTab(layout: Layout, tabId: string, patch: (t: TabSpec) => TabSpec): Layout {
  return { ...layout, tabs: layout.tabs.map((t) => (t.id === tabId ? patch(t) : t)) }
}

/** Every tab in pane order, as the single narrow-width strip shows them. */
export function orderedTabs(layout: Layout): TabSpec[] {
  const byId = new Map(layout.tabs.map((t) => [t.id, t]))
  return layout.panes.flatMap((p) => p.tabs.flatMap((id) => byId.get(id) ?? []))
}

/** A tab's label: the thing it looks at, short. */
export function tabTitle(tab: TabSpec): string {
  switch (tab.kind) {
    case 'logs':
      return tab.target.kind === 'container'
        ? tab.target.container
        : tab.target.service
          ? `${tab.target.stack}/${tab.target.service}`
          : tab.target.stack
    case 'shell':
      return tab.container
    case 'stats':
      return tab.target.kind === 'container' ? tab.target.container : tab.target.stack
  }
}
