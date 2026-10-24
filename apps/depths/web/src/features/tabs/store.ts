/**
 * Open tabs and their layout, per signed-in person (US4, T067).
 *
 * The layout is loaded when the session becomes ready and saved on every change, under
 * `depths:layout:<provider>:<subject>`. Leaving the signed-in state clears it from memory, so a
 * second person in the same tab never sees the first one's tabs. When the session ends by expiry
 * (signed out from a ready session), shell tabs are marked to offer a new session after signing in
 * again rather than starting one unasked; log and stats tabs simply reopen.
 *
 * This module is light (no xterm): the stack views import it for their "open a tab" buttons.
 */
import { create } from 'zustand'

import { useSession } from '@/app/session-store'
import type { Me } from '@/protocol/generated'

import {
  addTab,
  closeTab,
  EMPTY_LAYOUT,
  focusTab,
  loadLayout,
  moveTab,
  reorderTab,
  saveLayout,
  updateTab,
  type Layout,
  type TabSpec,
} from './layout'

/** A tab as asked for; the store gives it an id. */
export type NewTab = TabSpec extends infer T ? (T extends TabSpec ? Omit<T, 'id'> : never) : never

export interface TabsState {
  layout: Layout
  /** `<provider>:<subject>` of the person the layout belongs to; `null` when signed out. */
  subject: string | null
  openTab: (tab: NewTab) => void
  closeTab: (id: string) => void
  focusTab: (id: string) => void
  moveTab: (id: string, to: number | 'new') => void
  reorderTab: (id: string, delta: -1 | 1) => void
  updateTab: (id: string, patch: (t: TabSpec) => TabSpec) => void
  setSizes: (sizes: Record<string, number>) => void
  setOpen: (open: boolean) => void
}

let seq = 0
function newId(): string {
  seq += 1
  return `t${Date.now().toString(36)}${seq.toString(36)}${Math.floor(Math.random() * 1e6).toString(36)}`
}

export function subjectOf(me: Me): string {
  return `${me.identity.provider}:${me.identity.subject}`
}

/** Shell tabs are dropped while the shell feature is off (the server refuses them too). */
function allowed(layout: Layout, me: Me): Layout {
  if (me.features.shell) return layout
  let next = layout
  for (const t of layout.tabs) if (t.kind === 'shell') next = closeTab(next, t.id)
  return next
}

export const useTabs = create<TabsState>()((set, get) => {
  const edit = (f: (l: Layout) => Layout): void => {
    if (get().subject === null) return
    set((s) => ({ layout: f(s.layout) }))
  }
  return {
    layout: EMPTY_LAYOUT,
    subject: null,
    openTab: (tab) => {
      const me = useSession.getState().me
      if (tab.kind === 'shell' && !me?.features.shell) return
      edit((l) => addTab(l, { ...tab, id: newId() }))
    },
    closeTab: (id) => edit((l) => closeTab(l, id)),
    focusTab: (id) => edit((l) => focusTab(l, id)),
    moveTab: (id, to) => edit((l) => moveTab(l, id, to)),
    reorderTab: (id, delta) => edit((l) => reorderTab(l, id, delta)),
    updateTab: (id, patch) => edit((l) => updateTab(l, id, patch)),
    setSizes: (sizes) => edit((l) => ({ ...l, sizes: { ...l.sizes, ...sizes } })),
    setOpen: (open) => edit((l) => ({ ...l, open: open && l.tabs.length > 0 })),
  }
})

// Persist every change for the person it belongs to.
useTabs.subscribe((s, prev) => {
  if (s.subject !== null && s.subject === prev.subject && s.layout !== prev.layout) saveLayout(s.subject, s.layout)
})

function bind(status: string, me: Me | null, prevStatus: string): void {
  if (status === 'ready' && me) {
    const subject = subjectOf(me)
    if (useTabs.getState().subject !== subject) useTabs.setState({ subject, layout: allowed(loadLayout(subject), me) })
    return
  }
  if (prevStatus !== 'ready') return
  const { subject, layout } = useTabs.getState()
  if (subject !== null && status === 'signed_out' && useSession.getState().expired) {
    // The session ended under the tabs: shells are not resumable, so offer a new one later.
    let next = layout
    for (const t of layout.tabs) if (t.kind === 'shell') next = updateTab(next, t.id, (x) => ({ ...x, offerNew: true }))
    saveLayout(subject, next)
  }
  useTabs.setState({ subject: null, layout: EMPTY_LAYOUT })
}

useSession.subscribe((s, prev) => {
  if (s.status !== prev.status || s.me !== prev.me) bind(s.status, s.me, prev.status)
})
{
  const s = useSession.getState()
  bind(s.status, s.me, 'loading')
}
