/**
 * Virtualized resource list (T062): TanStack Table for the column and sort model, TanStack
 * Virtual for the rows, rendered as an ARIA grid of CSS-grid rows.
 *
 * - Columns come from the descriptor and hide by priority as the pane narrows
 *   (`computeVisibleColumns`); they never squash, and the body never scrolls sideways.
 * - Search (`PaneState.search`), sort (`PaneState.sort`), the open row (`PaneState.selection`),
 *   and the keyboard cursor (`PaneState.focusedRow`) all live in the store, so re-layout keeps them.
 * - Keyboard: ↑/↓/Home/End/PageUp/PageDown move the cursor, Enter opens the inspector,
 *   Space toggles the row's checkbox, Delete asks to delete the checked rows (else the cursor
 *   row), and Shift+F10 / the ContextMenu key open the cursor row's action menu.
 * - Actions (US3): right-click, the row's ⋯ button, or a long press on touch opens the row's
 *   action menu; a long press on a row without actions peeks the pressed cell's full text.
 */
import {
  createColumnHelper,
  createSortedRowModel,
  rowSortingFeature,
  tableFeatures,
  useTable,
  type SortingState,
  type Updater,
} from '@tanstack/react-table'
import { DotsThreeVerticalIcon } from '@phosphor-icons/react'
import { useVirtualizer } from '@tanstack/react-virtual'
import {
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
} from 'react'

import { RowActionMenu, type RowMenuState } from '@/components/ActionMenu'
import { actionTarget, promptAction } from '@/components/action-utils'
import { isExportable } from '@/features/manifest-export/kinds'
import { isAllowedNow } from '@/lib/capabilities'
import { computeVisibleColumns } from '@/layout/column-budget'
import { cn } from '@/lib/utils'
import type { AnyDescriptor, ColumnDef } from '@/resources'
import { mostRelevantRow } from '@/resources/relevance'
import { compileQuery } from '@/resources/search'
import { useStore, type ListRow, type PaneState } from '@/store'

import { Empty } from './shell/PaneStates'

const features = tableFeatures({
  rowSortingFeature,
  sortedRowModel: createSortedRowModel(),
})

type Row = ListRow
/** TanStack rows must be objects; list rows can be strings, so each is boxed with its key. */
interface Item {
  key: string
  row: Row
}
const helper = createColumnHelper<typeof features, Item>()

const NO_SELECTION: readonly string[] = []

/** A long press opened a menu: swallow the click the release would otherwise produce. */
let suppressClickUntil = 0

/** Opens the row menu at viewport point (x, y); `peek` is the long-pressed cell's text. */
type OpenRowMenu = (rowKey: string, x: number, y: number, peek?: string) => void

function cssPx(el: Element, name: string, fallback: number): number {
  const v = parseFloat(getComputedStyle(el).getPropertyValue(name))
  return Number.isFinite(v) && v > 0 ? v : fallback
}

function nameAndNamespace(row: Row): { name: string; namespace?: string | null } {
  if (typeof row === 'string') return { name: row }
  return {
    name: 'name' in row && typeof row.name === 'string' ? row.name : '',
    namespace: 'namespace' in row && typeof row.namespace === 'string' ? row.namespace : null,
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
  const toggleRowSelected = useStore((s) => s.toggleRowSelected)
  const focusList = useStore((s) => s.focusList)
  const checked = useStore((s) => s.selection[pane.id] ?? NO_SELECTION)
  const activeNamespace = useStore((s) => s.session.activeNamespace)

  // ---- filtering (client side; the backend also filters by namespace)
  const query = useDeferredValue(pane.search)
  // Boxed items are reused per row object: `reuseRows` keeps unchanged rows referentially stable
  // across list updates, so a 250 ms tick re-boxes only the rows that changed instead of 10k.
  // eslint-disable-next-line react-hooks/exhaustive-deps -- a new descriptor (rowKey) resets the cache
  const items = useMemo(() => new WeakMap<object, Item>(), [descriptor])
  const data = useMemo(() => {
    const out: Item[] = []
    const matches = compileQuery(query)
    for (const r of rows) {
      const nn = nameAndNamespace(r)
      if (activeNamespace !== null && descriptor.namespaced && nn.namespace !== activeNamespace) {
        continue
      }
      if (!matches(nn.name, nn.namespace)) continue
      let item = typeof r === 'object' ? items.get(r) : undefined
      if (!item) {
        item = { key: descriptor.rowKey(r), row: r }
        if (typeof r === 'object') items.set(r, item)
      }
      out.push(item)
    }
    return out
  }, [rows, query, activeNamespace, descriptor, items])

  // ---- column budget
  const bodyRef = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(0)
  useLayoutEffect(() => {
    const el = bodyRef.current
    if (!el) return
    const measure = () => {
      const check = cssPx(el, '--touch-target', 44)
      setWidth(Math.max(0, el.clientWidth - check))
    }
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])
  const visibleIds = useMemo(
    () => (width > 0 ? computeVisibleColumns(descriptor.columns, width) : null),
    [descriptor.columns, width],
  )
  const visibleCols = useMemo(
    () =>
      visibleIds
        ? descriptor.columns.filter((c) => visibleIds.includes(c.id))
        : descriptor.columns.filter((c) => c.priority === 1),
    [descriptor.columns, visibleIds],
  )
  const visibleKey = visibleCols.map((c) => c.id).join(',')
  useEffect(() => {
    const ids = visibleKey.split(',')
    const prev = useStore.getState().panes[pane.id]?.visibleColumns
    if (prev?.join(',') !== visibleKey) updatePane(pane.id, { visibleColumns: ids })
  }, [pane.id, updatePane, visibleKey])

  // ---- table model (sorting)
  const columns = useMemo(
    () =>
      helper.columns(
        visibleCols.map((c) =>
          helper.accessor((i) => c.text(i.row), {
            id: c.id,
            header: c.header,
            sortDescFirst: false,
            sortFn: (a, b) =>
              c.sort
                ? c.sort(a.original.row, b.original.row)
                : c.text(a.original.row).localeCompare(c.text(b.original.row)),
          }),
        ),
      ),
    [visibleCols],
  )
  const sorting = useMemo<SortingState>(() => (pane.sort ? [pane.sort] : []), [pane.sort])
  const onSortingChange = useCallback(
    (updater: Updater<SortingState>) => {
      const current = useStore.getState().panes[pane.id]?.sort
      const next = typeof updater === 'function' ? updater(current ? [current] : []) : updater
      updatePane(pane.id, { sort: next[0] })
    },
    [pane.id, updatePane],
  )
  const getRowId = useCallback((i: Item) => i.key, [])
  const table = useTable(
    { features, columns, data, getRowId, state: { sorting }, onSortingChange },
    (state) => ({ sorting: state.sorting }),
  )
  const modelRows = table.getRowModel().rows
  const keys = useMemo(() => modelRows.map((r) => r.id), [modelRows])

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
      modelRows.map((r) => r.original.row),
    )
    const row = key === undefined ? undefined : modelRows.find((r) => r.id === key)?.original.row
    // An empty (or fully filtered) list opens nothing, and the column stays waiting (FR-004).
    if (row === undefined) s.clearAutoOpen()
    else s.autoOpenStep(pane.id, descriptor.ref(row))
  }, [armed, modelRows, descriptor, pane.id, pane.route?.section])

  // ---- virtualization
  const rowHeight = useRowHeight(bodyRef)
  const virtualizer = useVirtualizer({
    count: modelRows.length,
    getScrollElement: () => bodyRef.current,
    estimateSize: () => rowHeight,
    getItemKey: (i) => keys[i] ?? i,
    overscan: 8,
    initialOffset: () => useStore.getState().panes[pane.id]?.scrollTop ?? 0,
  })
  useEffect(() => {
    virtualizer.measure()
  }, [rowHeight, virtualizer])
  // Re-mounting (phone Back, reflow) must land on the saved offset, not only seed the virtualizer.
  const restored = useRef(false)
  useLayoutEffect(() => {
    const el = bodyRef.current
    const saved = useStore.getState().panes[pane.id]?.scrollTop ?? 0
    if (!el || restored.current || saved <= 0) return
    el.scrollTop = saved
    // Content may not be tall enough yet (row height / rows still settling): retry next pass.
    if (el.scrollTop >= saved - 1) restored.current = true
  }, [pane.id, rowHeight, modelRows.length])

  const frame = useRef(0)
  const onScroll = useCallback(() => {
    const top = bodyRef.current?.scrollTop ?? 0
    cancelAnimationFrame(frame.current)
    frame.current = requestAnimationFrame(() => updatePane(pane.id, { scrollTop: top }))
  }, [pane.id, updatePane])
  useEffect(() => () => cancelAnimationFrame(frame.current), [])

  // ---- actions (US3)
  const checkedSet = useMemo(() => new Set(checked), [checked])
  const gridRef = useRef<HTMLDivElement>(null)
  const [menu, setMenu] = useState<RowMenuState | undefined>(undefined)
  // 040: exportable kinds get the row menu for "Export YAML…" even without mutating actions.
  const hasActions = descriptor.actions.length > 0 || isExportable(descriptor)
  const openMenu = useCallback<OpenRowMenu>((rowKey, x, y, peek) => {
    setMenu({ rowKey, x, y, ...(peek !== undefined ? { peek } : {}) })
  }, [])
  const menuRow = useMemo(
    () => (menu ? rows.find((r) => descriptor.rowKey(r) === menu.rowKey) : undefined),
    [descriptor, menu, rows],
  )
  const deleteAction = descriptor.actions.find((a) => a.id === 'delete')
  const promptDelete = () => {
    if (!deleteAction) return false
    const targets =
      checked.length > 0
        ? rows.filter((r) => checkedSet.has(descriptor.rowKey(r)))
        : rows.filter((r) => descriptor.rowKey(r) === pane.focusedRow)
    if (targets.length === 0 || (targets.length > 1 && !deleteAction.bulk)) return false
    // The Del key bypasses the gated menu item: it must pass the same capability check.
    if (!isAllowedNow(actionTarget(descriptor, deleteAction, targets))) return true
    promptAction(descriptor, deleteAction, targets, {
      paneId: pane.id,
      returnFocus: gridRef.current,
    })
    return true
  }
  const menuAtCursor = () => {
    const key = pane.focusedRow
    if (!hasActions || key === undefined) return false
    const el = gridRef.current?.querySelector(`[data-row-key="${CSS.escape(key)}"]`)
    const r = el?.getBoundingClientRect()
    openMenu(key, r ? r.left + 32 : 32, r ? r.bottom : 32)
    return true
  }

  // ---- keyboard
  const focusedIndex = pane.focusedRow ? keys.indexOf(pane.focusedRow) : -1
  const moveTo = useCallback(
    (i: number) => {
      const clamped = Math.max(0, Math.min(keys.length - 1, i))
      const key = keys[clamped]
      if (key === undefined) return
      updatePane(pane.id, { focusedRow: key })
      virtualizer.scrollToIndex(clamped, { align: 'auto' })
    },
    [keys, pane.id, updatePane, virtualizer],
  )
  const open = useCallback(
    (key: string) => {
      const row = modelRows.find((r) => r.id === key)
      if (row) openInspector(descriptor.ref(row.original.row), pane.id)
    },
    [descriptor, modelRows, openInspector, pane.id],
  )
  const page = Math.max(
    1,
    Math.floor((bodyRef.current?.clientHeight ?? rowHeight * 10) / rowHeight) - 1,
  )
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget) {
      // A clicked row or checkbox holds focus: still honour the action keys there.
      const inRow = e.target instanceof HTMLElement && e.target.closest('[role="row"]') !== null
      const actionKey = e.key === 'Delete' || e.key === 'ContextMenu' || e.key === 'F10'
      if (!inRow || !actionKey) return
    }
    switch (e.key) {
      case 'ArrowDown':
        moveTo(focusedIndex + 1)
        break
      case 'ArrowUp':
        moveTo(focusedIndex <= 0 ? 0 : focusedIndex - 1)
        break
      case 'Home':
        moveTo(0)
        break
      case 'End':
        moveTo(keys.length - 1)
        break
      case 'PageDown':
        moveTo(focusedIndex + page)
        break
      case 'PageUp':
        moveTo(focusedIndex - page)
        break
      case 'Enter':
        if (pane.focusedRow) open(pane.focusedRow)
        break
      case ' ':
        if (pane.focusedRow) toggleRowSelected(pane.id, pane.focusedRow)
        break
      case 'Delete':
        if (!promptDelete()) return
        break
      case 'ContextMenu':
        if (!menuAtCursor()) return
        break
      case 'F10':
        if (!e.shiftKey || !menuAtCursor()) return
        break
      default:
        return
    }
    e.preventDefault()
  }

  const template = useMemo(
    () =>
      [
        'var(--touch-target)',
        ...visibleCols.map((c) => `minmax(${String(c.minWidth)}px, ${String(c.grow ?? 1)}fr)`),
      ].join(' '),
    [visibleCols],
  )
  const cursorId = pane.focusedRow ? `${pane.id}-row-${String(focusedIndex)}` : undefined

  return (
    <div
      data-testid="resource-table"
      data-section={descriptor.section}
      data-row-count={modelRows.length}
      data-columns={visibleKey}
      className="flex h-full min-h-0 flex-col"
    >
      <div
        ref={gridRef}
        role="grid"
        aria-label={descriptor.label}
        aria-rowcount={modelRows.length + 1}
        aria-colcount={visibleCols.length + 1}
        aria-multiselectable="true"
        aria-activedescendant={focusedIndex >= 0 ? cursorId : undefined}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onFocus={() => {
          focusList(pane.id)
          if (focusedIndex < 0 && keys.length > 0 && pane.focusedRow === undefined) {
            // Start the cursor on the open row, else nowhere until an arrow key.
            if (pane.selection && keys.includes(pane.selection)) {
              updatePane(pane.id, { focusedRow: pane.selection })
            }
          }
        }}
        className="group/grid flex min-h-0 flex-1 flex-col outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset"
      >
        <div
          role="rowgroup"
          className="shrink-0 [scrollbar-gutter:stable] overflow-hidden border-b border-border bg-card"
        >
          <div
            role="row"
            aria-rowindex={1}
            className="grid h-[var(--row-height)] items-stretch"
            style={{ gridTemplateColumns: template }}
          >
            <div
              role="columnheader"
              aria-label="Select"
              className="flex items-center justify-center"
            >
              <span aria-hidden="true" className="text-xs text-muted-foreground">
                {checked.length > 0 ? String(checked.length) : ''}
              </span>
            </div>
            {table.getHeaderGroups()[0]?.headers.map((header) => {
              const col = visibleCols.find((c) => c.id === header.column.id)
              const dir = header.column.getIsSorted()
              return (
                <div
                  key={header.id}
                  role="columnheader"
                  aria-sort={dir === 'asc' ? 'ascending' : dir === 'desc' ? 'descending' : 'none'}
                  className="flex min-w-0"
                >
                  <button
                    type="button"
                    onClick={header.column.getToggleSortingHandler()}
                    className={cn(
                      'flex min-h-[var(--touch-target)] w-full min-w-0 items-center gap-1 px-2 font-display text-xs font-semibold tracking-[0.06em] uppercase outline-none',
                      'text-muted-foreground hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
                      col?.align === 'end' && 'justify-end',
                      dir && 'text-accent-neon hover:text-accent-neon',
                    )}
                  >
                    <span className="truncate">{col?.header ?? header.column.id}</span>
                    {dir && (
                      <span aria-hidden="true" className="shrink-0 text-xs">
                        {dir === 'asc' ? '▲' : '▼'}
                      </span>
                    )}
                  </button>
                </div>
              )
            })}
          </div>
        </div>
        <div
          role="rowgroup"
          ref={bodyRef}
          onScroll={onScroll}
          data-table-body=""
          className="relative min-h-0 flex-1 [scrollbar-gutter:stable] overflow-x-hidden overflow-y-auto"
        >
          {modelRows.length === 0 ? (
            <Empty
              title="No matches"
              detail={
                pane.search.trim()
                  ? `Nothing in ${descriptor.label.toLowerCase()} starts with “${pane.search.trim()}”.`
                  : `No ${descriptor.label.toLowerCase()} in this scope.`
              }
            />
          ) : (
            <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
              {virtualizer.getVirtualItems().map((item) => {
                const row = modelRows[item.index]
                if (!row) return null
                return (
                  <GridRow
                    key={row.id}
                    id={`${pane.id}-row-${String(item.index)}`}
                    paneId={pane.id}
                    rowKey={row.id}
                    index={item.index}
                    start={item.start}
                    row={row.original.row}
                    columns={visibleCols}
                    template={template}
                    open={row.id === pane.selection}
                    cursor={row.id === pane.focusedRow}
                    checked={checkedSet.has(row.id)}
                    onOpen={open}
                    onToggle={toggleRowSelected}
                    onMenu={hasActions ? openMenu : undefined}
                  />
                )
              })}
            </div>
          )}
        </div>
      </div>
      {menu && menuRow !== undefined && (
        <RowActionMenu
          descriptor={descriptor}
          row={menuRow}
          menu={menu}
          paneId={pane.id}
          returnFocus={gridRef.current}
          onClose={() => setMenu(undefined)}
        />
      )}
    </div>
  )
}

/** The density row height (`--row-height`), re-read when the width class changes it. */
function useRowHeight(ref: React.RefObject<HTMLElement | null>): number {
  const widthClass = useStore((s) => s.layout.widthClass)
  const [h, setH] = useState(28)
  useLayoutEffect(() => {
    const el = ref.current ?? document.documentElement
    setH(cssPx(el, '--row-height', 28))
  }, [ref, widthClass])
  return h
}

interface GridRowProps {
  id: string
  paneId: string
  rowKey: string
  index: number
  start: number
  row: Row
  columns: ColumnDef<Row>[]
  template: string
  open: boolean
  cursor: boolean
  checked: boolean
  onOpen: (key: string) => void
  onToggle: (paneId: string, key: string) => void
  /** Present when the kind has actions. */
  onMenu?: OpenRowMenu | undefined
}

const GridRow = memo(function GridRow({
  id,
  paneId,
  rowKey,
  index,
  start,
  row,
  columns,
  template,
  open,
  cursor,
  checked,
  onOpen,
  onToggle,
  onMenu,
}: GridRowProps) {
  const label = rowKey
  const onContextMenu = onMenu
    ? (e: MouseEvent) => {
        e.preventDefault()
        onMenu(rowKey, e.clientX, e.clientY)
      }
    : undefined
  return (
    <div
      id={id}
      role="row"
      aria-rowindex={index + 2}
      aria-selected={open}
      data-row-key={rowKey}
      data-focused={cursor ? 'true' : undefined}
      data-checked={checked ? 'true' : undefined}
      tabIndex={-1}
      onClick={() => {
        if (Date.now() < suppressClickUntil) return
        onOpen(rowKey)
      }}
      onContextMenu={onContextMenu}
      onKeyDown={(e) => {
        if (e.key === 'Enter') onOpen(rowKey)
      }}
      className={cn(
        'group/row absolute inset-x-0 top-0 grid h-[var(--row-height)] cursor-default items-center border-b border-border/40 text-sm',
        'hover:bg-muted/60',
        checked && !open && 'bg-secondary/70',
        open && 'bg-primary text-primary-foreground hover:bg-primary',
      )}
      style={{ gridTemplateColumns: template, transform: `translateY(${String(start)}px)` }}
    >
      <div role="gridcell" className="relative flex h-full items-center justify-center">
        {cursor && (
          <span
            aria-hidden="true"
            data-cursor=""
            className="absolute top-0 left-0 flex h-full items-center text-base leading-none text-accent-neon drop-shadow-[0_0_4px_var(--accent-neon)] group-aria-selected/row:text-primary-foreground"
          >
            ▌
          </span>
        )}
        <RowCheckbox
          checked={checked}
          label={`Select ${rowKey}`}
          onToggle={() => onToggle(paneId, rowKey)}
        />
      </div>
      {columns.map((c) => (
        <Cell
          key={c.id}
          col={c}
          row={row}
          onLongPress={onMenu ? (x, y, text) => onMenu(rowKey, x, y, text) : undefined}
        />
      ))}
      {onMenu && (
        <button
          type="button"
          tabIndex={-1}
          aria-label={`Actions for ${label}`}
          aria-haspopup="menu"
          data-testid="row-actions"
          onClick={(e) => {
            e.stopPropagation()
            const r = e.currentTarget.getBoundingClientRect()
            onMenu(rowKey, r.left, r.bottom)
          }}
          className={cn(
            'absolute top-0 right-0 hidden h-full min-h-[24px] w-[max(24px,var(--touch-target))] items-center justify-center bg-gradient-to-l from-background via-background/90 to-transparent text-muted-foreground outline-none hover:text-accent-neon [@media(pointer:fine)]:flex',
            'opacity-0 group-hover/row:opacity-100 group-data-[focused=true]/row:opacity-100',
            open &&
              'from-primary via-primary/90 text-primary-foreground hover:text-primary-foreground',
          )}
        >
          <DotsThreeVerticalIcon aria-hidden="true" weight="bold" className="size-4" />
        </button>
      )}
    </div>
  )
})

function RowCheckbox({
  checked,
  label,
  onToggle,
}: {
  checked: boolean
  label: string
  onToggle: () => void
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      aria-label={label}
      tabIndex={-1}
      onClick={(e) => {
        e.stopPropagation()
        onToggle()
      }}
      className="group/cb flex size-[var(--touch-target)] shrink-0 items-center justify-center outline-none"
    >
      <span
        aria-hidden="true"
        className={cn(
          'flex size-3.5 items-center justify-center border text-xs leading-none',
          checked
            ? 'border-accent-neon bg-accent-neon text-background'
            : 'border-muted-foreground group-hover/cb:border-foreground group-aria-selected/row:border-primary-foreground',
        )}
      >
        {checked ? '✓' : ''}
      </span>
    </button>
  )
}

const LONG_PRESS_MS = 500

/**
 * One cell: truncates with a `title`. A long press (touch) opens the row's action menu, which
 * shows the full cell text, or, for kinds without actions, peeks the full text in place.
 */
function Cell({
  col,
  row,
  onLongPress,
}: {
  col: ColumnDef<Row>
  row: Row
  onLongPress?: ((x: number, y: number, text: string) => void) | undefined
}) {
  const text = col.text(row)
  const [peek, setPeek] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const clear = () => clearTimeout(timer.current)
  const onPointerDown = (e: PointerEvent) => {
    if (e.pointerType !== 'touch') return
    clear()
    const { clientX: x, clientY: y } = e
    timer.current = setTimeout(() => {
      suppressClickUntil = Date.now() + 800
      if (onLongPress) onLongPress(x, y, text)
      else setPeek(true)
    }, LONG_PRESS_MS)
  }
  useEffect(() => clear, [])
  useEffect(() => {
    if (!peek) return
    const t = setTimeout(() => setPeek(false), 2500)
    return () => clearTimeout(t)
  }, [peek])

  const content: ReactNode = col.cell(row)
  return (
    <div
      role="gridcell"
      data-col={col.id}
      title={text}
      onPointerDown={onPointerDown}
      onPointerUp={clear}
      onPointerCancel={clear}
      onPointerLeave={clear}
      className={cn(
        'relative flex min-w-0 items-center px-2',
        col.align === 'end' && 'justify-end tabular-nums',
        col.id === 'name' && 'font-medium',
      )}
    >
      <span className="min-w-0 truncate">{content}</span>
      {peek && (
        <span
          role="tooltip"
          className="absolute bottom-full left-1 z-20 max-w-[80vw] border border-accent-neon bg-popover px-2 py-1 text-xs break-all text-popover-foreground shadow-lg"
        >
          {text}
        </span>
      )}
    </div>
  )
}
