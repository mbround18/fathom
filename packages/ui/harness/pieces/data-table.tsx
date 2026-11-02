import { useMemo, useState } from 'react'

import { DataTable, type Column, type SortState } from '../../src/kit'
import type { Fixture } from '../fixture'

interface Widget {
  id: string
  name: string
  size: number
  owner: string
}

const WIDGETS: readonly Widget[] = [
  { id: 'w1', name: 'Sprocket', size: 3, owner: 'ana' },
  { id: 'w2', name: 'Flange', size: 12, owner: 'bo' },
  { id: 'w3', name: 'Gasket', size: 7, owner: 'cy' },
  { id: 'w4', name: 'Bracket', size: 1, owner: 'ana' },
]

const COLUMNS: readonly Column<Widget>[] = [
  { key: 'name', header: 'Name', cell: (w) => w.name, sortable: true },
  { key: 'size', header: 'Size', cell: (w) => String(w.size), sortable: true },
  { key: 'owner', header: 'Owner', cell: (w) => w.owner },
]

function sorted(rows: readonly Widget[], sort: SortState | null): readonly Widget[] {
  if (!sort) return rows
  const by = sort.key === 'size' ? (a: Widget, b: Widget) => a.size - b.size : (a: Widget, b: Widget) => a.name.localeCompare(b.name)
  const out = [...rows].sort(by)
  return sort.dir === 'asc' ? out : out.reverse()
}

function Table({ withSort = false, withSelection = false, withActions = false, rows = WIDGETS }: {
  withSort?: boolean
  withSelection?: boolean
  withActions?: boolean
  rows?: readonly Widget[]
}) {
  const [sort, setSort] = useState<SortState | null>(null)
  const [checked, setChecked] = useState<ReadonlySet<string>>(new Set())
  const [last, setLast] = useState('none')
  const shown = useMemo(() => sorted(rows, withSort ? sort : null), [rows, sort, withSort])
  return (
    <div className="flex flex-col gap-2">
      <DataTable
        label="Widgets"
        columns={COLUMNS}
        rows={shown}
        rowKey={(w) => w.id}
        rowLabel={(w) => w.name}
        empty="No widgets yet."
        {...(withSort ? { sort: { value: sort, onChange: setSort } } : {})}
        {...(withSelection ? { selection: { value: checked, onChange: setChecked } } : {})}
        {...(withActions
          ? {
              rowActions: (w: Widget) => [
                { label: 'Rename', onSelect: () => setLast(`Rename ${w.name}`) },
                { label: 'Archive', onSelect: () => setLast(`Archive ${w.name}`), disabled: w.size > 10 },
                { label: 'Delete', onSelect: () => setLast(`Delete ${w.name}`), destructive: true },
              ],
            }
          : {})}
      />
      <p className="text-xs text-muted-foreground">
        Order: <span data-order>{shown.map((w) => w.name).join(',')}</span> · Checked:{' '}
        <span data-checked-keys>{[...checked].sort().join(',') || 'none'}</span> · Last action:{' '}
        <span data-last-action>{last}</span>
      </p>
    </div>
  )
}

const fixture: Fixture = {
  id: 'data-table',
  states: {
    plain: () => <Table />,
    empty: () => <Table rows={[]} withSort withSelection withActions />,
    sortable: () => <Table withSort />,
    selectable: () => <Table withSelection />,
    'with-actions': () => <Table withActions />,
  },
}

export default fixture
