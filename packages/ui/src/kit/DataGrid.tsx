/**
 * A virtualized list as an ARIA grid (spec 065 R8): the generic engine of Fathom's resource table,
 * in Keel's header, row and empty-state look. For long lists (10,000 rows stay fast); `DataTable`
 * is the semantic table for short ones.
 *
 * It is controlled. The app orders and filters `rows`, and owns `sort`, `selection`, the keyboard
 * `cursor` and the open row (`openKey`); the grid shows them and reports changes.
 *
 * - Columns hide by `priority` as the grid narrows (`computeVisibleColumns`); they never squash,
 *   and the body never scrolls sideways. Priority 1 always shows.
 * - Keyboard: ↑/↓/Home/End/PageUp/PageDown move the cursor, Enter opens the cursor row, Space
 *   checks it, and Shift+F10 or the ContextMenu key opens its menu. `onKeyDown` runs first, for the
 *   app's own keys (Delete); calling `preventDefault` there stops the grid's handling.
 * - Row menu: right-click, the row's ⋯ button (fine pointers) or a long press (touch). The kit
 *   shows `rowActions(row)` itself; an app with its own menu passes `onRowMenu` instead. A long
 *   press on a row with no menu peeks the pressed cell's full text.
 * - `empty` shows when there are no rows; `failed` replaces the rows when the list failed to load.
 */
import { DotsThreeVerticalIcon } from '@phosphor-icons/react'
import { useVirtualizer } from '@tanstack/react-virtual'
import {
  memo,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
  type Ref,
} from 'react'

import { Checkbox } from '../checkbox'
import { DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuTrigger } from '../dropdown-menu'

import { computeVisibleColumns } from './column-budget'
import {
  allState,
  ariaSort,
  EMPTY_CELL,
  HEAD,
  HEAD_CELL,
  ROW,
  ROW_CHECKED,
  ROW_OPEN,
  RowActionItems,
  SortButton,
  toggleAll,
  toggleKey,
  type RowActions,
  type TableSelection,
  type TableSort,
} from './table'

export interface GridColumn<T> {
  id: string
  header: string
  cell: (row: T) => ReactNode
  /** The cell's full text: its `title`, the long-press peek. */
  text: (row: T) => string
  /** 1 always shows; higher numbers hide first as the grid narrows. */
  priority: number
  /** The narrowest the column gets, in px. */
  minWidth: number
  /** Its share of the spare width (`fr`). Default 1. */
  grow?: number
  align?: 'start' | 'end'
  /** Its header sorts the grid, when the grid has `sort`. */
  sortable?: boolean
  /** Cell classes. */
  className?: string
}

/** The keyboard cursor, by row key. */
export interface GridCursor {
  value: string | null | undefined
  onChange: (key: string) => void
}

/** Where to open a row's menu: a viewport point, and the long-pressed cell's text. */
export interface RowMenuRequest {
  key: string
  x: number
  y: number
  peek?: string
}

const LONG_PRESS_MS = 500
const OVERSCAN = 8

/** A long press opened a menu: swallow the click its release would otherwise produce. */
let suppressClickUntil = 0

function cssPx(el: Element, name: string, fallback: number): number {
  const v = parseFloat(getComputedStyle(el).getPropertyValue(name))
  return Number.isFinite(v) && v > 0 ? v : fallback
}

interface Props<T> {
  label: string
  columns: readonly GridColumn<T>[]
  /** In display order: the app sorts and filters. */
  rows: readonly T[]
  rowKey: (row: T) => string
  /** How a row is named in "Select …" and "Actions for …". Default: `rowKey`. */
  rowLabel?: (row: T) => string
  /** `data-*` attributes for a row. */
  rowData?: (row: T) => Record<`data-${string}`, string>
  sort?: TableSort
  /** The checked rows: a checkbox column with select-all, and Space on the cursor row. */
  selection?: TableSelection
  /** The keyboard cursor. Without it the grid keeps its own. */
  cursor?: GridCursor
  /** The open row (`aria-current`), for example the one shown in a detail pane. */
  openKey?: string | null | undefined
  /** A click or Enter on a row. */
  onOpen?: (row: T, key: string) => void
  /** The kit's row menu. */
  rowActions?: RowActions<T>
  /** The app's own row menu, in place of `rowActions`. */
  onRowMenu?: (req: RowMenuRequest) => void
  /** The app's keys, before the grid's; `preventDefault` stops the grid's handling. */
  onKeyDown?: (e: KeyboardEvent<HTMLDivElement>, cursor: string | undefined) => void
  onFocus?: (e: FocusEvent<HTMLDivElement>) => void
  /** Shown in the body when there are no rows. */
  empty?: ReactNode
  /** Shown in place of the rows when the list failed to load. */
  failed?: ReactNode
  /** Row height in px. Default: the theme's `--row-height`. */
  rowHeight?: number
  /** Where the body starts scrolled, for example after a re-mount. */
  initialScrollTop?: number
  onScrollTopChange?: (top: number) => void
  /** The columns the budget shows, in declaration order. */
  onVisibleColumnsChange?: (ids: readonly string[]) => void
  /** The grid element (`role="grid"`), for returning focus to it. */
  ref?: Ref<HTMLDivElement>
  [data: `data-${string}`]: string | number | boolean | undefined
}

export function DataGrid<T>({
  label,
  columns,
  rows,
  rowKey,
  rowLabel = rowKey,
  rowData,
  sort,
  selection,
  cursor,
  openKey,
  onOpen,
  rowActions,
  onRowMenu,
  onKeyDown,
  onFocus,
  empty,
  failed,
  rowHeight: fixedHeight,
  initialScrollTop = 0,
  onScrollTopChange,
  onVisibleColumnsChange,
  ref,
  ...rest
}: Props<T>) {
  const baseId = useId()
  const [ownCursor, setOwnCursor] = useState<string | undefined>(undefined)
  const cursorKey = cursor ? (cursor.value ?? undefined) : ownCursor
  const setCursor = cursor ? cursor.onChange : setOwnCursor
  const keys = useMemo(() => rows.map(rowKey), [rows, rowKey])
  const indexOf = useMemo(() => new Map(keys.map((k, i) => [k, i])), [keys])
  const hasSelection = selection !== undefined

  // ---- column budget and row height
  const bodyRef = useRef<HTMLDivElement>(null)
  const gridRef = useRef<HTMLDivElement | null>(null)
  const [width, setWidth] = useState(0)
  const [cssRow, setCssRow] = useState(44)
  useLayoutEffect(() => {
    const el = bodyRef.current
    if (!el) return
    const measure = () => {
      const lead = hasSelection ? cssPx(el, '--touch-target', 44) : 0
      setWidth(Math.max(0, el.clientWidth - lead))
      setCssRow(cssPx(el, '--row-height', 44))
    }
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => {
      ro.disconnect()
    }
  }, [hasSelection])
  const rowHeight = fixedHeight ?? cssRow
  const visibleCols = useMemo(() => {
    if (width <= 0) return columns.filter((c) => c.priority === 1)
    const ids = new Set(computeVisibleColumns(columns, width))
    return columns.filter((c) => ids.has(c.id))
  }, [columns, width])
  const visibleKey = visibleCols.map((c) => c.id).join(',')
  const reportColumns = useRef(onVisibleColumnsChange)
  reportColumns.current = onVisibleColumnsChange
  useEffect(() => {
    reportColumns.current?.(visibleKey.split(','))
  }, [visibleKey])

  // ---- virtualization
  const virtualizer = useVirtualizer({
    count: failed === undefined ? rows.length : 0,
    getScrollElement: () => bodyRef.current,
    estimateSize: () => rowHeight,
    getItemKey: (i) => keys[i] ?? i,
    overscan: OVERSCAN,
    initialOffset: initialScrollTop,
  })
  useEffect(() => {
    virtualizer.measure()
  }, [rowHeight, virtualizer])
  // A re-mount must land on the saved offset, not only seed the virtualizer.
  const restored = useRef(false)
  useLayoutEffect(() => {
    const el = bodyRef.current
    if (!el || restored.current || initialScrollTop <= 0) return
    el.scrollTop = initialScrollTop
    // The content may not be tall enough yet (rows still arriving): retry on the next pass.
    if (el.scrollTop >= initialScrollTop - 1) restored.current = true
  }, [initialScrollTop, rowHeight, rows.length])
  const frame = useRef(0)
  const onScroll = useCallback(() => {
    if (!onScrollTopChange) return
    const top = bodyRef.current?.scrollTop ?? 0
    cancelAnimationFrame(frame.current)
    frame.current = requestAnimationFrame(() => {
      onScrollTopChange(top)
    })
  }, [onScrollTopChange])
  useEffect(
    () => () => {
      cancelAnimationFrame(frame.current)
    },
    [],
  )

  // ---- row menu
  const [menu, setMenu] = useState<RowMenuRequest | undefined>(undefined)
  const hasMenu = onRowMenu !== undefined || rowActions !== undefined
  const openMenu = useCallback(
    (req: RowMenuRequest) => {
      if (onRowMenu) onRowMenu(req)
      else setMenu(req)
    },
    [onRowMenu],
  )
  const menuIndex = menu ? indexOf.get(menu.key) : undefined
  const menuRow = menuIndex === undefined ? undefined : rows[menuIndex]
  const menuAtCursor = () => {
    if (!hasMenu || cursorKey === undefined || !indexOf.has(cursorKey)) return false
    const el = gridRef.current?.querySelector(`[data-row-key="${CSS.escape(cursorKey)}"]`)
    const r = el?.getBoundingClientRect()
    openMenu({ key: cursorKey, x: r ? r.left + 32 : 32, y: r ? r.bottom : 32 })
    return true
  }

  // ---- keyboard
  const focusedIndex = cursorKey === undefined ? -1 : (indexOf.get(cursorKey) ?? -1)
  const moveTo = (i: number) => {
    const clamped = Math.max(0, Math.min(keys.length - 1, i))
    const key = keys[clamped]
    if (key === undefined) return
    setCursor(key)
    virtualizer.scrollToIndex(clamped, { align: 'auto' })
  }
  const page = Math.max(1, Math.floor((bodyRef.current?.clientHeight ?? rowHeight * 10) / rowHeight) - 1)
  const handleKey = (e: KeyboardEvent<HTMLDivElement>) => {
    onKeyDown?.(e, cursorKey)
    if (e.defaultPrevented) return
    // The header's buttons keep their own keys; a clicked row's controls hand theirs to the grid.
    if (e.target !== e.currentTarget && !(e.target instanceof Element && e.target.closest('[role="row"][data-row-key]'))) return
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
      case 'Enter': {
        const row = focusedIndex >= 0 ? rows[focusedIndex] : undefined
        if (row === undefined || cursorKey === undefined || !onOpen) return
        onOpen(row, cursorKey)
        break
      }
      case ' ':
        if (!selection || cursorKey === undefined || focusedIndex < 0) return
        selection.onChange(toggleKey(selection.value, cursorKey))
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

  // Rows read the latest props through this, so a memoized row re-renders only when its own state
  // changes, not on every keystroke elsewhere in the grid.
  const latest = useRef({ rows, onOpen, setCursor, selection })
  latest.current = { rows, onOpen, setCursor, selection }
  const onRowClick = useCallback((index: number, key: string) => {
    if (Date.now() < suppressClickUntil) return
    const { rows: all, onOpen: open, setCursor: move } = latest.current
    move(key)
    // Keys go to the grid after a click, so the arrows carry on from the clicked row.
    gridRef.current?.focus({ preventScroll: true })
    const row = all[index]
    if (row !== undefined) open?.(row, key)
  }, [])
  const onRowToggle = useCallback((key: string) => {
    const sel = latest.current.selection
    sel?.onChange(toggleKey(sel.value, key))
  }, [])

  const leadWidth = hasSelection ? 'var(--touch-target)' : null
  const template = [
    ...(leadWidth ? [leadWidth] : []),
    ...visibleCols.map((c) => `minmax(${String(c.minWidth)}px, ${String(c.grow ?? 1)}fr)`),
  ].join(' ')
  const colCount = visibleCols.length + (hasSelection ? 1 : 0)
  const items = virtualizer.getVirtualItems()
  const cursorShown = focusedIndex >= 0 && items.some((i) => i.index === focusedIndex)
  const rowId = (i: number) => `${baseId}-row-${String(i)}`
  const status = failed !== undefined ? 'failed' : rows.length === 0 ? 'empty' : undefined

  return (
    <div {...rest} data-row-count={rows.length} data-columns={visibleKey} className="flex h-full min-h-0 min-w-0 flex-col">
      <div
        ref={(el) => {
          gridRef.current = el
          if (typeof ref === 'function') ref(el)
          else if (ref) ref.current = el
        }}
        role="grid"
        aria-label={label}
        aria-rowcount={rows.length + 1}
        aria-colcount={colCount}
        aria-multiselectable={hasSelection ? 'true' : undefined}
        aria-activedescendant={cursorShown ? rowId(focusedIndex) : undefined}
        tabIndex={0}
        onKeyDown={handleKey}
        onFocus={(e) => {
          if (cursorKey === undefined && openKey != null && indexOf.has(openKey)) setCursor(openKey)
          onFocus?.(e)
        }}
        className="flex min-h-0 flex-1 flex-col text-sm outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset"
      >
        <div role="rowgroup" className={`shrink-0 overflow-hidden border-b border-border [scrollbar-gutter:stable] ${HEAD}`}>
          <div role="row" aria-rowindex={1} className="grid items-stretch" style={{ gridTemplateColumns: template }}>
            {selection && (
              <div role="columnheader" className="flex items-center justify-center">
                <Checkbox
                  aria-label="Select all"
                  checked={allState(selection.value, keys)}
                  disabled={keys.length === 0}
                  onCheckedChange={() => {
                    selection.onChange(toggleAll(selection.value, keys))
                  }}
                />
              </div>
            )}
            {visibleCols.map((c) => {
              const sorts = sort !== undefined && c.sortable === true
              return (
                <div
                  key={c.id}
                  role="columnheader"
                  aria-sort={sorts ? ariaSort(sort.value, c.id) : undefined}
                  data-col={c.id}
                  className={`flex min-w-0 items-center ${HEAD_CELL} ${c.align === 'end' ? 'justify-end' : ''}`}
                >
                  {sorts ? (
                    <SortButton sort={sort} columnKey={c.id}>
                      {c.header}
                    </SortButton>
                  ) : (
                    <span className="truncate">{c.header}</span>
                  )}
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
          {status ? (
            <div role="row" aria-rowindex={2}>
              <div
                role="gridcell"
                aria-colspan={colCount}
                data-grid-status={status}
                className={status === 'empty' ? EMPTY_CELL : 'px-2 py-4'}
              >
                {status === 'failed' ? failed : empty}
              </div>
            </div>
          ) : (
            <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
              {items.map((item) => {
                const row = rows[item.index]
                const key = keys[item.index]
                if (row === undefined || key === undefined) return null
                return (
                  <GridRow
                    key={key}
                    id={rowId(item.index)}
                    rowKey={key}
                    label={rowLabel(row)}
                    index={item.index}
                    start={item.start}
                    height={rowHeight}
                    row={row}
                    data={rowData?.(row)}
                    columns={visibleCols}
                    template={template}
                    open={key === openKey}
                    cursor={key === cursorKey}
                    checked={selection ? selection.value.has(key) : undefined}
                    onClick={onRowClick}
                    onToggle={onRowToggle}
                    onMenu={hasMenu ? openMenu : undefined}
                  />
                )
              })}
            </div>
          )}
        </div>
      </div>
      {menu && rowActions && menuRow !== undefined && (
        <DropdownMenu
          open
          modal={false}
          onOpenChange={(o) => {
            if (!o) setMenu(undefined)
          }}
        >
          <DropdownMenuTrigger asChild>
            <span aria-hidden className="pointer-events-none fixed size-0" style={{ left: menu.x, top: menu.y }} />
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="start"
            className="w-auto max-w-[80vw]"
            aria-label={`Actions for ${rowLabel(menuRow)}`}
            onCloseAutoFocus={(e) => {
              e.preventDefault()
              gridRef.current?.focus({ preventScroll: true })
            }}
          >
            {menu.peek !== undefined && <DropdownMenuLabel className="break-all text-foreground">{menu.peek}</DropdownMenuLabel>}
            <RowActionItems actions={rowActions(menuRow)} />
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  )
}

interface RowProps<T> {
  id: string
  rowKey: string
  label: string
  index: number
  start: number
  height: number
  row: T
  data: Record<`data-${string}`, string> | undefined
  columns: readonly GridColumn<T>[]
  template: string
  open: boolean
  cursor: boolean
  /** `undefined` when the grid has no selection. */
  checked: boolean | undefined
  onClick: (index: number, key: string) => void
  onToggle: (key: string) => void
  onMenu: ((req: RowMenuRequest) => void) | undefined
}

const GridRow = memo(function GridRow<T>({
  id,
  rowKey,
  label,
  index,
  start,
  height,
  row,
  data,
  columns,
  template,
  open,
  cursor,
  checked,
  onClick,
  onToggle,
  onMenu,
}: RowProps<T>) {
  const glyph = cursor && (
    // Decoration over the first cell: it must not take the clicks meant for the checkbox.
    <span
      aria-hidden="true"
      data-cursor=""
      className="pointer-events-none absolute top-0 left-0 flex h-full items-center text-base leading-none text-accent-neon drop-shadow-[0_0_4px_var(--accent-neon)]"
    >
      ▌
    </span>
  )
  const last = columns.length - 1
  return (
    <div
      {...data}
      id={id}
      role="row"
      aria-rowindex={index + 2}
      // Checked is the grid's selection (`aria-selected`); open is where the user is (`aria-current`).
      aria-selected={checked}
      aria-current={open ? 'true' : undefined}
      data-row-key={rowKey}
      data-open={open ? 'true' : undefined}
      data-focused={cursor ? 'true' : undefined}
      data-checked={checked ? 'true' : undefined}
      tabIndex={-1}
      onClick={() => {
        onClick(index, rowKey)
      }}
      onKeyDown={(e) => {
        // A row only has focus when it was clicked; the grid handles every other key.
        if (e.key !== 'Enter' || e.target !== e.currentTarget) return
        e.preventDefault()
        onClick(index, rowKey)
      }}
      onContextMenu={
        onMenu
          ? (e: MouseEvent) => {
              e.preventDefault()
              onMenu({ key: rowKey, x: e.clientX, y: e.clientY })
            }
          : undefined
      }
      className={`group/row absolute inset-x-0 top-0 grid cursor-default items-center ${ROW} ${open ? ROW_OPEN : checked ? ROW_CHECKED : ''} ${cursor && !open ? 'bg-muted/30' : ''}`}
      style={{ gridTemplateColumns: template, height, transform: `translateY(${String(start)}px)` }}
    >
      {checked !== undefined && (
        <div role="gridcell" className="relative flex h-full items-center justify-center">
          {glyph}
          <Checkbox
            tabIndex={-1}
            aria-label={`Select ${label}`}
            checked={checked}
            onClick={(e) => {
              e.stopPropagation()
            }}
            onCheckedChange={() => {
              onToggle(rowKey)
            }}
          />
        </div>
      )}
      {columns.map((c, i) => (
        <Cell
          key={c.id}
          col={c}
          row={row}
          onLongPress={
            onMenu
              ? (x, y, text) => {
                  onMenu({ key: rowKey, x, y, peek: text })
                }
              : undefined
          }
        >
          {checked === undefined && i === 0 && glyph}
          {onMenu && i === last && (
            <button
              type="button"
              tabIndex={-1}
              aria-label={`Actions for ${label}`}
              aria-haspopup="menu"
              data-testid="row-actions"
              onClick={(e) => {
                e.stopPropagation()
                const r = e.currentTarget.getBoundingClientRect()
                onMenu({ key: rowKey, x: r.left, y: r.bottom })
              }}
              className="absolute top-0 right-0 hidden h-full w-[max(24px,var(--touch-target))] items-center justify-center bg-gradient-to-l from-background via-background/90 to-transparent text-muted-foreground opacity-0 outline-none group-hover/row:opacity-100 group-data-[focused=true]/row:opacity-100 hover:text-accent-neon [@media(pointer:fine)]:flex"
            >
              <DotsThreeVerticalIcon aria-hidden="true" weight="bold" className="size-4" />
            </button>
          )}
        </Cell>
      ))}
    </div>
  )
}) as <T>(props: RowProps<T>) => ReactNode

/**
 * One cell: truncates, with its full text as `title`. A long press (touch) opens the row's menu
 * with that text, or, for a row with no menu, peeks the text in place.
 */
function Cell<T>({
  col,
  row,
  onLongPress,
  children,
}: {
  col: GridColumn<T>
  row: T
  onLongPress: ((x: number, y: number, text: string) => void) | undefined
  children?: ReactNode
}) {
  const text = col.text(row)
  const [peek, setPeek] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const clear = () => {
    clearTimeout(timer.current)
  }
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
    const t = setTimeout(() => {
      setPeek(false)
    }, 2500)
    return () => {
      clearTimeout(t)
    }
  }, [peek])

  return (
    <div
      role="gridcell"
      data-col={col.id}
      title={text}
      onPointerDown={onPointerDown}
      onPointerUp={clear}
      onPointerCancel={clear}
      onPointerLeave={clear}
      className={`relative flex h-full min-w-0 items-center px-2 ${col.align === 'end' ? 'justify-end tabular-nums' : ''} ${col.className ?? ''}`}
    >
      {children}
      <span className="min-w-0 truncate">{col.cell(row)}</span>
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
