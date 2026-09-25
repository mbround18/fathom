/**
 * Pane pinning and rearranging (spec 001 T086). Two list slots can exchange their view state
 * (section, filter, selection, sort, scroll): the header menu's "Pin to slot N" and drag-and-drop
 * between list pane headers both call `swapListPanes`. Slot sizes persist separately
 * (SlotGrid -> store.setProfile -> SaveUiConfig).
 */
import { useStore, type PaneId, type PaneState } from '@/store'

type ViewState = Pick<
  PaneState,
  'route' | 'search' | 'selection' | 'scrollTop' | 'sort' | 'focusedRow' | 'visibleColumns'
>

const view = (p: PaneState): ViewState => ({
  route: p.route,
  search: p.search,
  selection: p.selection,
  scrollTop: p.scrollTop,
  sort: p.sort,
  focusedRow: p.focusedRow,
  visibleColumns: p.visibleColumns,
})

/** Other list panes currently in a slot, as `[paneId, slotIndex]`. */
export function listSlotTargets(from: PaneId): [PaneId, number][] {
  const { layout, panes } = useStore.getState()
  return layout.slotPanes
    .map((id, i): [PaneId, number] => [id, i])
    .filter(([id]) => id !== from && panes[id]?.kind === 'list')
}

/** Exchange the view state of two list panes. Returns false when either is not a list. */
export function swapListPanes(a: PaneId, b: PaneId): boolean {
  const { panes, updatePane } = useStore.getState()
  const pa = panes[a]
  const pb = panes[b]
  if (!pa || !pb || a === b || pa.kind !== 'list' || pb.kind !== 'list') return false
  const va = view(pa)
  const vb = view(pb)
  updatePane(a, vb)
  updatePane(b, va)
  return true
}
