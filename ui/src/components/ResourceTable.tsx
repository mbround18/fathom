/**
 * Virtualized resource list (T062): TanStack Table for the column and sort model, TanStack
 * Virtual for the rows, rendered as an ARIA grid of CSS-grid rows.
 *
 * - Columns come from the descriptor and hide by priority as the pane narrows
 *   (`computeVisibleColumns`); they never squash, and the body never scrolls sideways.
 * - Search (`PaneState.search`), sort (`PaneState.sort`), the open row (`PaneState.selection`),
 *   and the keyboard cursor (`PaneState.focusedRow`) all live in the store, so re-layout keeps them.
 * - Keyboard: ↑/↓/Home/End/PageUp/PageDown move the cursor, Enter opens the inspector,
 *   Space toggles the row's checkbox.
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
  type PointerEvent,
  type ReactNode,
} from 'react'

import { computeVisibleColumns } from '@/layout/column-budget'
import { cn } from '@/lib/utils'
import type { AnyDescriptor, ColumnDef } from '@/resources'
import { matchesQuery } from '@/resources/search'
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
  const data = useMemo(() => {
    const out: Item[] = []
    for (const r of rows) {
      const nn = nameAndNamespace(r)
      if (activeNamespace !== null && descriptor.namespaced && nn.namespace !== activeNamespace) {
        continue
      }
      if (matchesQuery(nn, query)) out.push({ key: descriptor.rowKey(r), row: r })
    }
    return out
  }, [rows, query, activeNamespace, descriptor])

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

  const frame = useRef(0)
  const onScroll = useCallback(() => {
    const top = bodyRef.current?.scrollTop ?? 0
    cancelAnimationFrame(frame.current)
    frame.current = requestAnimationFrame(() => updatePane(pane.id, { scrollTop: top }))
  }, [pane.id, updatePane])
  useEffect(() => () => cancelAnimationFrame(frame.current), [])

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
    if (e.target !== e.currentTarget) return
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
  const checkedSet = useMemo(() => new Set(checked), [checked])
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
              <span aria-hidden="true" className="text-[0.6rem] text-muted-foreground">
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
                      'flex min-h-[var(--touch-target)] w-full min-w-0 items-center gap-1 px-2 font-display text-[0.68rem] font-semibold tracking-[0.06em] uppercase outline-none',
                      'text-muted-foreground hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
                      col?.align === 'end' && 'justify-end',
                      dir && 'text-accent-neon hover:text-accent-neon',
                    )}
                  >
                    <span className="truncate">{col?.header ?? header.column.id}</span>
                    {dir && (
                      <span aria-hidden="true" className="shrink-0 text-[0.6rem]">
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
                  />
                )
              })}
            </div>
          )}
        </div>
      </div>
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
}: GridRowProps) {
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
      onClick={() => onOpen(rowKey)}
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
        <Cell key={c.id} col={c} row={row} />
      ))}
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
          'flex size-3.5 items-center justify-center border text-[0.6rem] leading-none',
          checked
            ? 'border-accent-neon bg-accent-neon text-background'
            : 'border-muted-foreground/60 group-hover/cb:border-foreground group-aria-selected/row:border-primary-foreground/70',
        )}
      >
        {checked ? '✓' : ''}
      </span>
    </button>
  )
}

const LONG_PRESS_MS = 500

/** One cell: truncates with a `title`, and a long press (touch) peeks the full text. */
function Cell({ col, row }: { col: ColumnDef<Row>; row: Row }) {
  const text = col.text(row)
  const [peek, setPeek] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const clear = () => clearTimeout(timer.current)
  const onPointerDown = (e: PointerEvent) => {
    if (e.pointerType !== 'touch') return
    clear()
    timer.current = setTimeout(() => setPeek(true), LONG_PRESS_MS)
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
