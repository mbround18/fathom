import { useMemo, useState } from 'react'

import { DataGrid, type GridColumn, type SortState } from '../../src/kit'
import type { Fixture } from '../fixture'

interface Widget {
  id: string
  name: string
  size: number
  owner: string
  note: string
}

const WIDGETS: readonly Widget[] = [
  { id: 'w1', name: 'Sprocket', size: 3, owner: 'ana', note: 'Spare kept in the north shed' },
  { id: 'w2', name: 'Flange', size: 12, owner: 'bo', note: 'Ordered twice by mistake' },
  { id: 'w3', name: 'Gasket', size: 7, owner: 'cy', note: 'Rubber, replace yearly' },
  { id: 'w4', name: 'Bracket', size: 1, owner: 'ana', note: 'Holds the shelf up' },
]

const OWNERS = ['ana', 'bo', 'cy']
const MANY: readonly Widget[] = Array.from({ length: 10_000 }, (_, i) => ({
  id: `m${String(i)}`,
  name: `Widget ${String(i).padStart(5, '0')}`,
  size: (i * 7919) % 1000,
  owner: OWNERS[i % 3] ?? 'ana',
  note: `Batch ${String(Math.floor(i / 100))}`,
}))

const COLUMNS: readonly GridColumn<Widget>[] = [
  { id: 'name', header: 'Name', cell: (w) => w.name, text: (w) => w.name, priority: 1, minWidth: 120, grow: 2, sortable: true },
  { id: 'size', header: 'Size', cell: (w) => String(w.size), text: (w) => String(w.size), priority: 2, minWidth: 64, align: 'end', sortable: true },
  { id: 'owner', header: 'Owner', cell: (w) => w.owner, text: (w) => w.owner, priority: 3, minWidth: 80 },
  { id: 'note', header: 'Note', cell: (w) => w.note, text: (w) => w.note, priority: 4, minWidth: 220, grow: 3 },
]

function sorted(rows: readonly Widget[], sort: SortState | null): readonly Widget[] {
  if (!sort) return rows
  const by = sort.key === 'size' ? (a: Widget, b: Widget) => a.size - b.size : (a: Widget, b: Widget) => a.name.localeCompare(b.name)
  const out = [...rows].sort(by)
  return sort.dir === 'asc' ? out : out.reverse()
}

function Grid({
  rows = WIDGETS,
  full = false,
  failed = false,
}: {
  rows?: readonly Widget[]
  /** Sort, selection and the kit's row menu. */
  full?: boolean
  failed?: boolean
}) {
  const [sort, setSort] = useState<SortState | null>(null)
  const [checked, setChecked] = useState<ReadonlySet<string>>(new Set())
  const [cursor, setCursor] = useState<string | undefined>(undefined)
  const [open, setOpen] = useState<string | undefined>(undefined)
  const [last, setLast] = useState('none')
  const shown = useMemo(() => sorted(rows, full ? sort : null), [rows, sort, full])
  return (
    <div className="flex flex-col gap-2">
      <div className="h-[28rem] border border-border">
        <DataGrid
          label="Widgets"
          columns={COLUMNS}
          rows={shown}
          rowKey={(w) => w.id}
          rowLabel={(w) => w.name}
          cursor={{ value: cursor, onChange: setCursor }}
          openKey={open}
          onOpen={(_, key) => {
            setOpen(key)
          }}
          empty="No widgets yet."
          {...(failed ? { failed: <span role="alert">Widgets failed to load.</span> } : {})}
          {...(full
            ? {
                sort: { value: sort, onChange: setSort },
                selection: { value: checked, onChange: setChecked },
                rowActions: (w: Widget) => [
                  { label: 'Rename', onSelect: () => setLast(`Rename ${w.name}`) },
                  { label: 'Archive', onSelect: () => setLast(`Archive ${w.name}`), disabled: w.size > 10 },
                  { label: 'Delete', onSelect: () => setLast(`Delete ${w.name}`), destructive: true },
                ],
              }
            : {})}
          data-testid="widget-grid"
        />
      </div>
      <p className="text-xs text-muted-foreground">
        Order: <span data-order>{shown.slice(0, 4).map((w) => w.name).join(',')}</span> · Cursor:{' '}
        <span data-cursor-key>{cursor ?? 'none'}</span> · Open: <span data-open-key>{open ?? 'none'}</span> · Checked:{' '}
        <span data-checked-keys>{checked.size > 5 ? `${String(checked.size)} rows` : [...checked].sort().join(',') || 'none'}</span>{' '}
        · Last action: <span data-last-action>{last}</span>
      </p>
    </div>
  )
}

const fixture: Fixture = {
  id: 'data-grid',
  states: {
    plain: () => <Grid />,
    full: () => <Grid full />,
    empty: () => <Grid rows={[]} full />,
    failed: () => <Grid full failed />,
    'ten-thousand': () => <Grid rows={MANY} full />,
  },
}

export default fixture
