import { describe, expect, it } from 'vitest'

import { mostRelevantRow } from '@/resources/relevance'
import type { ColumnDef } from '@/resources/types'

interface Row {
  name: string
  status: string
}

const nameColumn: ColumnDef<Row> = {
  id: 'name',
  header: 'Name',
  priority: 1,
  minWidth: 100,
  cell: (r) => r.name,
  text: (r) => r.name,
}

const statusColumn: ColumnDef<Row> = {
  id: 'status',
  header: 'Status',
  priority: 1,
  minWidth: 80,
  status: true,
  cell: (r) => r.status,
  text: (r) => r.status,
}

const withStatus = { columns: [nameColumn, statusColumn], rowKey: (r: Row) => r.name }
const withoutStatus = { columns: [nameColumn], rowKey: (r: Row) => r.name }

const row = (name: string, status: string): Row => ({ name, status })

describe('mostRelevantRow (044, FR-003)', () => {
  it('gives undefined for an empty list (FR-004)', () => {
    expect(mostRelevantRow(withStatus, [])).toBeUndefined()
  })

  it('prefers a failure over an earlier warning', () => {
    const rows = [row('a', 'Pending'), row('b', 'CrashLoopBackOff'), row('c', 'Error')]
    expect(mostRelevantRow(withStatus, rows)).toBe('b')
  })

  it('prefers a warning over a later healthy row', () => {
    const rows = [row('a', 'Running'), row('b', 'Pending'), row('c', 'Running')]
    expect(mostRelevantRow(withStatus, rows)).toBe('b')
  })

  it('gives the first row when every row is healthy', () => {
    const rows = [row('a', 'Running'), row('b', 'Succeeded')]
    expect(mostRelevantRow(withStatus, rows)).toBe('a')
  })

  it('gives the first row when the kind has no status column', () => {
    const rows = [row('a', 'Error'), row('b', 'Running')]
    expect(mostRelevantRow(withoutStatus, rows)).toBe('a')
  })

  it('does not treat an unrecognised status as unhealthy', () => {
    // `muted` is "no health to report", not a problem, so the first row still wins.
    const rows = [row('a', 'Opaque'), row('b', 'Running')]
    expect(mostRelevantRow(withStatus, rows)).toBe('a')
  })

  it('follows the list order the operator sees, not the array it came from', () => {
    const rows = [row('a', 'Running'), row('b', 'Pending'), row('c', 'Error')]
    // Sorted or filtered differently, the answer changes with the order.
    expect(mostRelevantRow(withStatus, [...rows].reverse())).toBe('c')
    expect(mostRelevantRow(withStatus, rows.slice(0, 2))).toBe('b')
    expect(mostRelevantRow(withStatus, rows.slice(0, 1))).toBe('a')
  })
})
