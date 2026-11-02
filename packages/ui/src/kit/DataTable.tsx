/**
 * A list of records (spec 060 R10): a table from 640 px up, a card list below it, so a phone never
 * scrolls sideways. One layout is rendered at a time (from a media query), so each row exists once
 * in the page. `filter` sits above the rows; `actions(row)` are each row's buttons; `selected(row)`
 * marks the open one.
 *
 * With no rows the table keeps its headers and says why it is empty in one row, so a table a page
 * names is always there to find; the card list shows a dashed empty box instead.
 *
 * Optional and controlled (spec 065 R8): `sort` makes the `sortable` columns' headers buttons with
 * `aria-sort`; `selection` adds a checkbox column with select-all; `rowActions(row)` adds a ⋯ menu
 * at the end of each row. The app orders `rows` itself. Without them the output is as it was.
 */
import { DotsThreeIcon } from '@phosphor-icons/react'
import { useSyncExternalStore, type ReactNode } from 'react'

import { Button } from '../button'
import { Checkbox } from '../checkbox'
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '../dropdown-menu'

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
  type RowAction,
  type RowActions,
  type TableSelection,
  type TableSort,
} from './table'

export interface Column<T> {
  key: string
  header: string
  /** An info icon after the header (a glossary term's `TermTip`), in the table layout only. */
  tip?: ReactNode
  cell: (row: T) => ReactNode
  /** Cell classes in the table layout. */
  className?: string
  /** Its header sorts the table, when the table has `sort`. */
  sortable?: boolean
}

const WIDE = '(min-width: 640px)'

/** `null` where there is no `matchMedia` (jsdom), which counts as wide so unit tests see the table. */
function query(): MediaQueryList | null {
  return typeof globalThis.matchMedia === 'function' ? globalThis.matchMedia(WIDE) : null
}

function subscribe(notify: () => void): () => void {
  const mq = query()
  mq?.addEventListener('change', notify)
  return () => mq?.removeEventListener('change', notify)
}

function wide(): boolean {
  return query()?.matches ?? true
}

function useWide(): boolean {
  return useSyncExternalStore(subscribe, wide, () => true)
}

export function DataTable<T>({
  label,
  columns,
  rows,
  rowKey,
  rowData,
  actions,
  filter,
  selected,
  empty,
  sort,
  selection,
  rowActions,
  rowLabel = rowKey,
}: {
  label: string
  columns: readonly Column<T>[]
  rows: readonly T[]
  rowKey: (row: T) => string
  /** `data-*` attributes for a row, for tests and styling. */
  rowData?: (row: T) => Record<`data-${string}`, string>
  actions?: (row: T) => ReactNode
  filter?: ReactNode
  /** Whether a row is the one open below the table. */
  selected?: (row: T) => boolean
  empty: ReactNode
  /** The sorted column, shown on the `sortable` headers. The app sorts `rows`. */
  sort?: TableSort
  /** The checked rows, by `rowKey`: a checkbox column with select-all. */
  selection?: TableSelection
  /** Each row's ⋯ menu. */
  rowActions?: RowActions<T>
  /** How a row is named in "Select …" and "Actions for …". Default: `rowKey`. */
  rowLabel?: (row: T) => string
}) {
  const isWide = useWide()
  const hasActions = actions !== undefined || rowActions !== undefined
  const keys = selection ? rows.map(rowKey) : []
  const sortable = sort ? columns.filter((c) => c.sortable === true) : []
  const checked = (r: T) => selection?.value.has(rowKey(r)) === true
  const rowCheckbox = (r: T) =>
    selection && (
      <Checkbox
        aria-label={`Select ${rowLabel(r)}`}
        checked={checked(r)}
        onCheckedChange={() => {
          selection.onChange(toggleKey(selection.value, rowKey(r)))
        }}
      />
    )
  const selectAll = (labelled: boolean) =>
    selection && (
      <Checkbox
        {...(labelled ? { 'aria-label': 'Select all' } : {})}
        checked={allState(selection.value, keys)}
        disabled={keys.length === 0}
        onCheckedChange={() => {
          selection.onChange(toggleAll(selection.value, keys))
        }}
      />
    )
  return (
    <div className="flex min-w-0 flex-col gap-2">
      {filter !== undefined && <div className="flex min-w-0 flex-wrap items-center gap-2">{filter}</div>}
      {!isWide && rows.length > 0 && (selection !== undefined || sortable.length > 0) && (
        <div data-table-tools className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          {selection && (
            <label className="inline-flex min-h-6 items-center gap-2">
              {selectAll(false)}
              Select all
            </label>
          )}
          {sort && sortable.length > 0 && (
            <span role="group" aria-label="Sort by" className="inline-flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
              <span aria-hidden>Sort by</span>
              {sortable.map((c) => (
                <SortButton key={c.key} sort={sort} columnKey={c.key}>
                  {c.header}
                  <span className="sr-only">{sortedText(ariaSort(sort.value, c.key))}</span>
                </SortButton>
              ))}
            </span>
          )}
        </div>
      )}
      {rows.length === 0 && !isWide ? (
        <div data-pane-state="empty" className="border border-dashed border-border px-3 py-4 text-xs text-muted-foreground">
          {empty}
        </div>
      ) : isWide ? (
        <table aria-label={label} className="w-full table-fixed border-collapse text-left text-sm">
          <thead className={HEAD}>
            <tr className="border-b border-border">
              {selection && (
                <th scope="col" className="w-10 px-2 py-2">
                  <span className="flex items-center">{selectAll(true)}</span>
                </th>
              )}
              {columns.map((c) => {
                const sorts = sort !== undefined && c.sortable === true
                const header = sorts ? (
                  <SortButton sort={sort} columnKey={c.key}>
                    {c.header}
                  </SortButton>
                ) : (
                  c.header
                )
                return (
                  <th
                    key={c.key}
                    scope="col"
                    aria-sort={sorts ? ariaSort(sort.value, c.key) : undefined}
                    className={`${HEAD_CELL} ${c.className ?? ''}`}
                  >
                    {c.tip === undefined ? (
                      header
                    ) : (
                      <span className="inline-flex items-center gap-1.5">
                        {header}
                        {c.tip}
                      </span>
                    )}
                  </th>
                )
              })}
              {hasActions && (
                <th scope="col" className={`${actions ? 'w-40' : 'w-12'} px-2 py-1.5`}>
                  <span className="sr-only">Actions</span>
                </th>
              )}
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr>
                <td
                  colSpan={columns.length + (hasActions ? 1 : 0) + (selection ? 1 : 0)}
                  data-pane-state="empty"
                  className={EMPTY_CELL}
                >
                  {empty}
                </td>
              </tr>
            )}
            {rows.map((r) => (
              <tr
                key={rowKey(r)}
                {...rowData?.(r)}
                aria-current={selected?.(r) ? 'true' : undefined}
                data-checked={checked(r) ? 'true' : undefined}
                className={`${ROW} ${selected?.(r) ? ROW_OPEN : ''}${checked(r) ? ` ${ROW_CHECKED}` : ''}`}
              >
                {selection && (
                  <td className="px-2 py-1.5 align-top">
                    <span className="flex min-h-5 items-center">{rowCheckbox(r)}</span>
                  </td>
                )}
                {columns.map((c) => (
                  <td key={c.key} className={`px-2 py-1.5 align-top break-words ${c.className ?? ''}`}>
                    {c.cell(r)}
                  </td>
                ))}
                {hasActions && (
                  <td className="px-2 py-1 text-right align-top">
                    <span className="inline-flex flex-wrap justify-end gap-1">
                      {actions?.(r)}
                      {rowActions && <RowMenu label={rowLabel(r)} actions={() => rowActions(r)} />}
                    </span>
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <ul aria-label={label} className="flex min-w-0 flex-col gap-2">
          {rows.map((r) => (
            <li
              key={rowKey(r)}
              {...rowData?.(r)}
              aria-current={selected?.(r) ? 'true' : undefined}
              data-checked={checked(r) ? 'true' : undefined}
              className={`flex min-w-0 flex-col gap-1 border p-2 text-xs ${selected?.(r) ? 'border-accent-neon' : 'border-border'}${checked(r) ? ` ${ROW_CHECKED}` : ''}`}
            >
              {(selection !== undefined || rowActions !== undefined) && (
                <div className="flex min-w-0 items-center gap-2">
                  {rowCheckbox(r)}
                  {rowActions && (
                    <span className="ml-auto">
                      <RowMenu label={rowLabel(r)} actions={() => rowActions(r)} />
                    </span>
                  )}
                </div>
              )}
              <dl className="grid min-w-0 grid-cols-[minmax(0,7rem)_minmax(0,1fr)] gap-x-2 gap-y-0.5">
                {columns.map((c) => (
                  <div key={c.key} className="contents">
                    <dt className="text-muted-foreground">{c.header}</dt>
                    <dd className="min-w-0 break-words">{c.cell(r)}</dd>
                  </div>
                ))}
              </dl>
              {actions && <div className="flex flex-wrap justify-end gap-1 pt-1">{actions(r)}</div>}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function sortedText(s: 'ascending' | 'descending' | 'none'): string {
  return s === 'none' ? '' : `, sorted ${s}`
}

/** A row's ⋯ button and its menu. Its entries are read when it opens. */
function RowMenu({ label, actions }: { label: string; actions: () => readonly RowAction[] }) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-xs" aria-label={`Actions for ${label}`} data-row-actions>
          <DotsThreeIcon aria-hidden weight="bold" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-auto">
        <RowActionItems actions={actions()} />
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
