/**
 * A list of records (spec 060 R10): a table from 640 px up, a card list below it, so a phone never
 * scrolls sideways. One layout is rendered at a time (from a media query), so each row exists once
 * in the page. `filter` sits above the rows; `actions(row)` are each row's buttons; `selected(row)`
 * marks the open one.
 *
 * With no rows the table keeps its headers and says why it is empty in one row, so a table a page
 * names is always there to find; the card list shows a dashed empty box instead.
 */
import { useSyncExternalStore, type ReactNode } from 'react'

export interface Column<T> {
  key: string
  header: string
  /** An info icon after the header (a glossary term's `TermTip`), in the table layout only. */
  tip?: ReactNode
  cell: (row: T) => ReactNode
  /** Cell classes in the table layout. */
  className?: string
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
}) {
  const isWide = useWide()
  return (
    <div className="flex min-w-0 flex-col gap-2">
      {filter !== undefined && <div className="flex min-w-0 flex-wrap items-center gap-2">{filter}</div>}
      {rows.length === 0 && !isWide ? (
        <div data-pane-state="empty" className="border border-dashed border-border px-3 py-4 text-xs text-muted-foreground">
          {empty}
        </div>
      ) : isWide ? (
        <table aria-label={label} className="w-full table-fixed border-collapse text-left text-sm">
          <thead className="text-muted-foreground">
            <tr className="border-b border-border">
              {columns.map((c) => (
                <th key={c.key} scope="col" className={`px-2 py-2 font-medium ${c.className ?? ''}`}>
                  {c.tip === undefined ? (
                    c.header
                  ) : (
                    <span className="inline-flex items-center gap-1.5">
                      {c.header}
                      {c.tip}
                    </span>
                  )}
                </th>
              ))}
              {actions && (
                <th scope="col" className="w-40 px-2 py-1.5">
                  <span className="sr-only">Actions</span>
                </th>
              )}
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr>
                <td
                  colSpan={columns.length + (actions ? 1 : 0)}
                  data-pane-state="empty"
                  className="border-b border-dashed border-border px-2 py-4 text-muted-foreground"
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
                className={`border-b border-border hover:bg-muted/30 ${selected?.(r) ? 'bg-accent-neon/10 shadow-[inset_2px_0_0_var(--color-accent-neon)]' : ''}`}
              >
                {columns.map((c) => (
                  <td key={c.key} className={`px-2 py-1.5 align-top break-words ${c.className ?? ''}`}>
                    {c.cell(r)}
                  </td>
                ))}
                {actions && (
                  <td className="px-2 py-1 text-right align-top">
                    <span className="inline-flex flex-wrap justify-end gap-1">{actions(r)}</span>
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
              className={`flex min-w-0 flex-col gap-1 border p-2 text-xs ${selected?.(r) ? 'border-accent-neon' : 'border-border'}`}
            >
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
