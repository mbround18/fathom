/**
 * What `DataTable` and `DataGrid` share (spec 065 R8, data-model.md): the controlled sort,
 * selection and row-action shapes, the header sort button, the ⋯ menu's items, and Keel's header,
 * row and empty-state classes, so both tables look alike.
 *
 * Both tables are controlled: they show `sort` and `selection` and report changes, and the app
 * orders and filters `rows` itself.
 */
import { CaretDownIcon, CaretUpIcon } from '@phosphor-icons/react'
import type { ReactNode } from 'react'

import { DropdownMenuItem } from '../dropdown-menu'

export interface SortState {
  key: string
  dir: 'asc' | 'desc'
}

export interface TableSort {
  value: SortState | null
  onChange: (s: SortState | null) => void
}

export interface TableSelection {
  value: ReadonlySet<string>
  onChange: (s: Set<string>) => void
}

export interface RowAction {
  label: string
  onSelect: () => void
  destructive?: boolean
  disabled?: boolean
}

export type RowActions<T> = (row: T) => readonly RowAction[]

/** Keel's table header text. */
export const HEAD = 'text-muted-foreground'
/** Keel's header cell. */
export const HEAD_CELL = 'px-2 py-2 font-medium'
/** Keel's row: a hairline under it and a faint hover. */
export const ROW = 'border-b border-border hover:bg-muted/30'
/** The open row: a neon wash and a neon bar on its left edge. */
export const ROW_OPEN = 'bg-accent-neon/10 shadow-[inset_2px_0_0_var(--color-accent-neon)]'
/** A checked row. */
export const ROW_CHECKED = 'bg-muted/50'
/** The row that says why a table is empty. */
export const EMPTY_CELL = 'border-b border-dashed border-border px-2 py-4 text-muted-foreground'

/** A header click cycles its column through ascending, descending, then unsorted. */
export function nextSort(current: SortState | null, key: string): SortState | null {
  if (current?.key !== key) return { key, dir: 'asc' }
  return current.dir === 'asc' ? { key, dir: 'desc' } : null
}

export function ariaSort(current: SortState | null, key: string): 'ascending' | 'descending' | 'none' {
  if (current?.key !== key) return 'none'
  return current.dir === 'asc' ? 'ascending' : 'descending'
}

/** `set` with `key` flipped. */
export function toggleKey(set: ReadonlySet<string>, key: string): Set<string> {
  const next = new Set(set)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  return next
}

/** Whether all, some or none of `keys` are in `set`, as a checkbox shows it. */
export function allState(set: ReadonlySet<string>, keys: readonly string[]): boolean | 'indeterminate' {
  let n = 0
  for (const k of keys) if (set.has(k)) n++
  if (n === 0) return false
  return n === keys.length ? true : 'indeterminate'
}

/** Select-all: adds every key, or removes them all when they are already all in. */
export function toggleAll(set: ReadonlySet<string>, keys: readonly string[]): Set<string> {
  const next = new Set(set)
  if (allState(set, keys) === true) for (const k of keys) next.delete(k)
  else for (const k of keys) next.add(k)
  return next
}

/** A sortable column's header: the name and the direction it is sorted in. */
export function SortButton({
  sort,
  columnKey,
  className = '',
  children,
}: {
  sort: TableSort
  columnKey: string
  className?: string
  children: ReactNode
}) {
  const dir = sort.value?.key === columnKey ? sort.value.dir : undefined
  return (
    <button
      type="button"
      data-sort-key={columnKey}
      onClick={() => {
        sort.onChange(nextSort(sort.value, columnKey))
      }}
      className={`inline-flex min-h-6 min-w-0 items-center gap-1 text-left font-medium outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring ${dir ? 'text-foreground' : ''} ${className}`}
    >
      <span className="min-w-0 truncate">{children}</span>
      {dir === 'asc' && <CaretUpIcon aria-hidden weight="bold" className="size-3 shrink-0 text-accent-neon" />}
      {dir === 'desc' && <CaretDownIcon aria-hidden weight="bold" className="size-3 shrink-0 text-accent-neon" />}
    </button>
  )
}

/** A row's ⋯ menu entries. */
export function RowActionItems({ actions }: { actions: readonly RowAction[] }) {
  return actions.map((a) => (
    <DropdownMenuItem
      key={a.label}
      variant={a.destructive ? 'destructive' : 'default'}
      disabled={a.disabled}
      onSelect={a.onSelect}
    >
      {a.label}
    </DropdownMenuItem>
  ))
}
