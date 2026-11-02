import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { DataTable, type Column } from '../../src/kit'
import { allState, nextSort, toggleAll, toggleKey } from '../../src/kit/table'

interface W {
  id: string
  name: string
}
const ROWS: readonly W[] = [
  { id: 'a', name: 'Alpha' },
  { id: 'b', name: 'Beta' },
]
const COLS: readonly Column<W>[] = [{ key: 'name', header: 'Name', cell: (w) => w.name, sortable: true }]

describe('table helpers', () => {
  it('cycles a column through ascending, descending, then none', () => {
    expect(nextSort(null, 'name')).toEqual({ key: 'name', dir: 'asc' })
    expect(nextSort({ key: 'name', dir: 'asc' }, 'name')).toEqual({ key: 'name', dir: 'desc' })
    expect(nextSort({ key: 'name', dir: 'desc' }, 'name')).toBeNull()
    expect(nextSort({ key: 'size', dir: 'desc' }, 'name')).toEqual({ key: 'name', dir: 'asc' })
  })

  it('toggles one key, and all keys', () => {
    expect([...toggleKey(new Set(['a']), 'b')]).toEqual(['a', 'b'])
    expect([...toggleKey(new Set(['a']), 'a')]).toEqual([])
    expect(allState(new Set(['a']), ['a', 'b'])).toBe('indeterminate')
    expect([...toggleAll(new Set(['a']), ['a', 'b'])].sort()).toEqual(['a', 'b'])
    expect([...toggleAll(new Set(['a', 'b', 'x']), ['a', 'b'])]).toEqual(['x'])
  })
})

describe('DataTable', () => {
  it('without sort, selection or rowActions renders no extra controls', () => {
    render(<DataTable label="Widgets" columns={COLS} rows={ROWS} rowKey={(w) => w.id} empty="None." />)
    const table = screen.getByRole('table', { name: 'Widgets' })
    expect(within(table).queryAllByRole('button')).toEqual([])
    expect(within(table).queryAllByRole('checkbox')).toEqual([])
    expect(within(table).getByRole('columnheader', { name: 'Name' })).not.toHaveAttribute('aria-sort')
  })

  it('reports sort and selection changes', async () => {
    const onSort = vi.fn()
    const onSelect = vi.fn()
    render(
      <DataTable
        label="Widgets"
        columns={COLS}
        rows={ROWS}
        rowKey={(w) => w.id}
        rowLabel={(w) => w.name}
        empty="None."
        sort={{ value: null, onChange: onSort }}
        selection={{ value: new Set(), onChange: onSelect }}
      />,
    )
    await userEvent.click(screen.getByRole('button', { name: 'Name' }))
    expect(onSort).toHaveBeenCalledWith({ key: 'name', dir: 'asc' })
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select Beta' }))
    expect([...(onSelect.mock.calls[0]?.[0] as Set<string>)]).toEqual(['b'])
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select all' }))
    expect([...(onSelect.mock.calls[1]?.[0] as Set<string>)]).toEqual(['a', 'b'])
  })
})
