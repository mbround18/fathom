/**
 * A pane's resource list (T062, spec 065 T028): Fathom's adapter over the kit's `DataGrid`. The
 * grid is the engine (virtualized ARIA grid, column budget, roving cursor, long press); this file
 * maps Fathom's store, descriptors, compiled search, capabilities and row menu onto it.
 *
 * - Search (`PaneState.search`), sort (`PaneState.sort`), the open row (`PaneState.selection`),
 *   the keyboard cursor (`PaneState.focusedRow`) and the scroll offset all live in the store, so
 *   re-layout keeps them. The checked rows are `store.selection[paneId]`.
 * - Keyboard: the grid's (↑/↓/Home/End/PageUp/PageDown, Enter opens the inspector, Space checks,
 *   Shift+F10 / ContextMenu opens the row menu), plus Delete, which asks to delete the checked
 *   rows (else the cursor row).
 * - Actions (US3): right-click, the row's ⋯ button, or a long press opens Fathom's
 *   `RowActionMenu`; a long press on a kind without actions peeks the pressed cell's full text.
 */
import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from 'react'

import {
  DataGrid,
  EmptyState,
  NoMatch,
  type GridColumn,
  type RowMenuRequest,
  type SortState,
} from '@companion/ui/kit'

import { RowActionMenu, type RowMenuState } from '@/components/ActionMenu'
import { actionTarget, promptAction } from '@/components/action-utils'
import { isExportable } from '@/features/manifest-export/kinds'
import { isAllowedNow } from '@/lib/capabilities'
import type { AnyDescriptor, ColumnDef } from '@/resources'
import { mostRelevantRow } from '@/resources/relevance'
import { compileQuery } from '@/resources/search'
import { useStore, type ListRow, type PaneState } from '@/store'

type Row = ListRow
/** List rows can be strings, so each is boxed with its key. */
interface Item {
  key: string
  row: Row
}

const NO_SELECTION: readonly string[] = []
const itemKey = (i: Item) => i.key

/**
 * Boxed items are reused per row object, so unchanged rows stay referentially stable across list
 * updates and a 250 ms tick re-boxes only the rows that changed instead of 10k. The cache is per
 * descriptor, since the box carries that descriptor's row key.
 */
const BOXES = new WeakMap<AnyDescriptor, WeakMap<object, Item>>()
function boxesFor(descriptor: AnyDescriptor): WeakMap<object, Item> {
  let boxes = BOXES.get(descriptor)
  if (!boxes) {
    boxes = new WeakMap()
    BOXES.set(descriptor, boxes)
  }
  return boxes
}

function sortState(key: string, desc: boolean): SortState {
  return { key, dir: desc ? 'desc' : 'asc' }
}

function nameAndNamespace(row: Row): { name: string; namespace?: string | null } {
  if (typeof row === 'string') return { name: row }
  return {
    name: 'name' in row && typeof row.name === 'string' ? row.name : '',
    namespace: 'namespace' in row && typeof row.namespace === 'string' ? row.namespace : null,
  }
}

function gridColumn(c: ColumnDef<Row>): GridColumn<Item> {
  return {
    id: c.id,
    header: c.header,
    cell: (i) => c.cell(i.row),
    text: (i) => c.text(i.row),
    priority: c.priority,
    minWidth: c.minWidth,
    ...(c.grow !== undefined ? { grow: c.grow } : {}),
    ...(c.align !== undefined ? { align: c.align } : {}),
    sortable: true,
    ...(c.id === 'name' ? { className: 'font-medium' } : {}),
  }
}

interface ResourceTableProps {
  pane: PaneState
  descriptor: AnyDescriptor
  rows: readonly Row[]
}

export function ResourceTable({ pane, descriptor, rows }: ResourceTableProps) {
  const updatePane = useStore((s) => s.updatePane)
  const openInspector = useStore((s) => s.openInspector)
  const setSelection = useStore((s) => s.setSelection)
  const focusList = useStore((s) => s.focusList)
  const checked = useStore((s) => s.selection[pane.id] ?? NO_SELECTION)
  const activeNamespace = useStore((s) => s.session.activeNamespace)

  // ---- filtering (client side; the backend also filters by namespace)
  const query = useDeferredValue(pane.search)
  const filtered = useMemo(() => {
    const boxes = boxesFor(descriptor)
    const out: Item[] = []
    const matches = compileQuery(query)
    for (const r of rows) {
      const nn = nameAndNamespace(r)
      if (activeNamespace !== null && descriptor.namespaced && nn.namespace !== activeNamespace) {
        continue
      }
      if (!matches(nn.name, nn.namespace)) continue
      let item = typeof r === 'object' ? boxes.get(r) : undefined
      if (!item) {
        item = { key: descriptor.rowKey(r), row: r }
        if (typeof r === 'object') boxes.set(r, item)
      }
      out.push(item)
    }
    return out
  }, [rows, query, activeNamespace, descriptor])

  // ---- sorting: a column's comparator, else its text; ties keep the backend's order
  const sortCol = pane.sort ? descriptor.columns.find((c) => c.id === pane.sort?.id) : undefined
  const desc = pane.sort?.desc ?? false
  const items = useMemo(() => {
    if (!sortCol) return filtered
    const cmp = sortCol.sort
      ? (a: Item, b: Item) => sortCol.sort?.(a.row, b.row) ?? 0
      : (a: Item, b: Item) => sortCol.text(a.row).localeCompare(sortCol.text(b.row))
    const sign = desc ? -1 : 1
    return filtered.slice().sort((a, b) => sign * cmp(a, b))
  }, [filtered, sortCol, desc])
  const sort = useMemo(
    () => ({
      value: pane.sort ? sortState(pane.sort.id, pane.sort.desc) : null,
      onChange: (s: SortState | null) => {
        updatePane(pane.id, { sort: s ? { id: s.key, desc: s.dir === 'desc' } : undefined })
      },
    }),
    [pane.id, pane.sort, updatePane],
  )

  const columns = useMemo(() => descriptor.columns.map(gridColumn), [descriptor.columns])
  const onVisibleColumnsChange = useCallback(
    (ids: readonly string[]) => {
      const prev = useStore.getState().panes[pane.id]?.visibleColumns
      if (prev?.join(',') !== ids.join(',')) updatePane(pane.id, { visibleColumns: [...ids] })
    },
    [pane.id, updatePane],
  )

  // ---- 044 US1: the auto-open of the section's most relevant item (FR-003)
  // This component only renders once the list has its first page, so the arm `navigate` left is
  // consumed here rather than on every list update: one O(n) scan per section open, off the
  // render path and never before there is something to choose from (SC-003). The rows scanned are
  // the ones the operator can see, in their order, so their sort and filter decide.
  const armed = useStore((s) => s.layout.pendingAutoOpen?.paneId === pane.id)
  useEffect(() => {
    if (!armed) return
    const s = useStore.getState()
    // The arm belongs to one section click; a pane showing something else has moved on.
    if (s.layout.pendingAutoOpen?.route.section !== pane.route?.section) return
    const key = mostRelevantRow(
      descriptor,
      items.map((i) => i.row),
    )
    const row = key === undefined ? undefined : items.find((i) => i.key === key)?.row
    // An empty (or fully filtered) list opens nothing, and the column stays waiting (FR-004).
    if (row === undefined) s.clearAutoOpen()
    else s.autoOpenStep(pane.id, descriptor.ref(row))
  }, [armed, items, descriptor, pane.id, pane.route?.section])

  // ---- store-backed grid state
  const cursor = useMemo(
    () => ({
      value: pane.focusedRow,
      onChange: (key: string) => {
        updatePane(pane.id, { focusedRow: key })
      },
    }),
    [pane.id, pane.focusedRow, updatePane],
  )
  const checkedSet = useMemo(() => new Set(checked), [checked])
  const selection = useMemo(
    () => ({
      value: checkedSet,
      onChange: (next: Set<string>) => {
        setSelection(pane.id, [...next])
      },
    }),
    [checkedSet, pane.id, setSelection],
  )
  const onOpen = useCallback(
    (i: Item) => {
      openInspector(descriptor.ref(i.row), pane.id)
    },
    [descriptor, openInspector, pane.id],
  )
  // Read once: the grid restores it on mount and reports scrolls back.
  const [initialScrollTop] = useState(() => useStore.getState().panes[pane.id]?.scrollTop ?? 0)
  const onScrollTopChange = useCallback(
    (top: number) => {
      updatePane(pane.id, { scrollTop: top })
    },
    [pane.id, updatePane],
  )

  // ---- actions (US3)
  const gridRef = useRef<HTMLDivElement>(null)
  const [menu, setMenu] = useState<RowMenuState | undefined>(undefined)
  // Where focus goes when the menu closes, captured when it opens (refs aren't read in render).
  const [menuReturn, setMenuReturn] = useState<HTMLElement | null>(null)
  // 040: exportable kinds get the row menu for "Export YAML…" even without mutating actions.
  const hasActions = descriptor.actions.length > 0 || isExportable(descriptor)
  const onRowMenu = useCallback((req: RowMenuRequest) => {
    setMenuReturn(gridRef.current)
    setMenu({ rowKey: req.key, x: req.x, y: req.y, ...(req.peek !== undefined ? { peek: req.peek } : {}) })
  }, [])
  const menuRow = useMemo(
    () => (menu ? rows.find((r) => descriptor.rowKey(r) === menu.rowKey) : undefined),
    [descriptor, menu, rows],
  )
  const deleteAction = descriptor.actions.find((a) => a.id === 'delete')
  const promptDelete = (cursorKey: string | undefined) => {
    if (!deleteAction) return false
    const targets =
      checked.length > 0
        ? rows.filter((r) => checkedSet.has(descriptor.rowKey(r)))
        : rows.filter((r) => descriptor.rowKey(r) === cursorKey)
    if (targets.length === 0 || (targets.length > 1 && !deleteAction.bulk)) return false
    // The Del key bypasses the gated menu item: it must pass the same capability check.
    if (!isAllowedNow(actionTarget(descriptor, deleteAction, targets))) return true
    promptAction(descriptor, deleteAction, targets, {
      paneId: pane.id,
      returnFocus: gridRef.current,
    })
    return true
  }
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>, cursorKey: string | undefined) => {
    if (e.key !== 'Delete') return
    // On the grid, or a clicked row's control; never the header's sort buttons.
    const inRow = e.target instanceof HTMLElement && e.target.closest('[role="row"][data-row-key]')
    if (e.target !== e.currentTarget && !inRow) return
    if (promptDelete(cursorKey)) e.preventDefault()
  }

  const search = pane.search.trim()
  const label = descriptor.label.toLowerCase()
  return (
    <>
      <DataGrid<Item>
        ref={gridRef}
        data-testid="resource-table"
        data-section={descriptor.section}
        label={descriptor.label}
        columns={columns}
        rows={items}
        rowKey={itemKey}
        sort={sort}
        selection={selection}
        cursor={cursor}
        openKey={pane.selection}
        onOpen={onOpen}
        {...(hasActions ? { onRowMenu } : {})}
        onKeyDown={onKeyDown}
        onFocus={() => {
          focusList(pane.id)
        }}
        initialScrollTop={initialScrollTop}
        onScrollTopChange={onScrollTopChange}
        onVisibleColumnsChange={onVisibleColumnsChange}
        empty={
          search ? (
            <NoMatch
              what={label}
              message={`Nothing in ${label} starts with “${search}”.`}
              onClear={() => {
                updatePane(pane.id, { search: '' })
              }}
            />
          ) : (
            <EmptyState title={`No ${label} in this scope.`} />
          )
        }
      />
      {menu && menuRow !== undefined && (
        <RowActionMenu
          descriptor={descriptor}
          row={menuRow}
          menu={menu}
          paneId={pane.id}
          returnFocus={menuReturn}
          onClose={() => {
            setMenu(undefined)
          }}
        />
      )}
    </>
  )
}
