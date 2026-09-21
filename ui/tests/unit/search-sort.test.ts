/**
 * T058: search and sort decide which rows the operator sees (constitution III), so they are
 * unit tested apart from the table. Ports the query rules of tests/gui_search_filter.rs.
 */
import { describe, expect, it } from 'vitest'

import {
  compareAge,
  compareNumber,
  compareStatus,
  compareText,
  matchesQuery,
  sortRows,
  statusTone,
} from '@/resources/search'

import { PODS_500 } from '../fixtures'

const row = (name: string, namespace = 'default') => ({ name, namespace })

describe('matchesQuery', () => {
  it('matches everything for an empty or blank query', () => {
    expect(matchesQuery(row('api-1'), '')).toBe(true)
    expect(matchesQuery(row('api-1'), '   ')).toBe(true)
  })

  it('matches a name prefix, case-insensitively', () => {
    expect(matchesQuery(row('web-abc'), 'web-')).toBe(true)
    expect(matchesQuery(row('web-abc'), 'WEB-')).toBe(true)
    expect(matchesQuery(row('Web-abc'), 'web')).toBe(true)
  })

  it('does not match a name infix (prefix only)', () => {
    expect(matchesQuery(row('api-web-1'), 'web')).toBe(false)
  })

  it('matches by namespace', () => {
    expect(matchesQuery(row('api-1', 'payments'), 'payments')).toBe(true)
    expect(matchesQuery(row('api-1', 'payments'), 'PAY')).toBe(true)
    expect(matchesQuery(row('api-1', 'payments'), 'checkout')).toBe(false)
  })

  it('matches namespace/name', () => {
    expect(matchesQuery(row('web-1', 'payments'), 'payments/web')).toBe(true)
    expect(matchesQuery(row('web-1', 'search'), 'payments/web')).toBe(false)
  })

  it('works for cluster-scoped rows without a namespace', () => {
    expect(matchesQuery({ name: 'node-1' }, 'node')).toBe(true)
    expect(matchesQuery({ name: 'node-1' }, 'default')).toBe(false)
  })

  it('filters the 500-pod fixture to its web- pods and restores on clear', () => {
    const web = PODS_500.filter((p) => matchesQuery(p, 'web-'))
    expect(web.length).toBeGreaterThan(0)
    expect(web.length).toBeLessThan(PODS_500.length)
    expect(web.every((p) => p.name.startsWith('web-'))).toBe(true)
    expect(PODS_500.filter((p) => matchesQuery(p, 'WEB-'))).toEqual(web)
    expect(PODS_500.filter((p) => matchesQuery(p, ''))).toHaveLength(PODS_500.length)
  })
})

describe('comparators', () => {
  it('compareText is locale-aware and numeric-aware', () => {
    expect(compareText('a', 'b')).toBeLessThan(0)
    expect(compareText('pod-2', 'pod-10')).toBeLessThan(0)
    expect(compareText('B', 'a')).toBeGreaterThan(0)
    expect(compareText('x', 'x')).toBe(0)
  })

  it('compareNumber orders ascending', () => {
    expect(compareNumber(1, 2)).toBeLessThan(0)
    expect(compareNumber(10, 2)).toBeGreaterThan(0)
    expect(compareNumber(3, 3)).toBe(0)
  })

  it('compareAge sorts youngest first (a later timestamp is a smaller age)', () => {
    const older = '2026-09-19T08:00:00Z'
    const newer = '2026-09-20T08:00:00Z'
    expect(compareAge(newer, older)).toBeLessThan(0)
    expect(compareAge(older, newer)).toBeGreaterThan(0)
    expect(compareAge(older, older)).toBe(0)
  })

  it('compareAge puts unknown timestamps last', () => {
    expect(compareAge('', '2026-09-20T08:00:00Z')).toBeGreaterThan(0)
    expect(compareAge('2026-09-20T08:00:00Z', 'garbage')).toBeLessThan(0)
  })

  it('compareStatus orders failures before warnings before healthy, then by text', () => {
    const phases = ['Running', 'Succeeded', 'Pending', 'Failed', 'Unknown', 'CrashLoopBackOff']
    const sorted = [...phases].sort(compareStatus)
    expect(sorted.indexOf('Failed')).toBeLessThan(sorted.indexOf('Pending'))
    expect(sorted.indexOf('CrashLoopBackOff')).toBeLessThan(sorted.indexOf('Pending'))
    expect(sorted.indexOf('Pending')).toBeLessThan(sorted.indexOf('Running'))
    expect(sorted.indexOf('Unknown')).toBeLessThan(sorted.indexOf('Running'))
    expect(compareStatus('Running', 'Running')).toBe(0)
  })

  it('statusTone never returns a colour without a matching glyph', () => {
    for (const phase of ['Running', 'Pending', 'Failed', 'Succeeded', 'Unknown', 'weird']) {
      const tone = statusTone(phase)
      expect(['ok', 'warn', 'err', 'muted']).toContain(tone.tone)
      expect(tone.glyph).toMatch(/^[●▲✖○]$/)
    }
  })
})

describe('sortRows', () => {
  const rows = [
    { name: 'b', n: 2 },
    { name: 'a', n: 3 },
    { name: 'c', n: 1 },
  ]
  const byN = (x: { n: number }, y: { n: number }) => compareNumber(x.n, y.n)

  it('sorts ascending and descending without mutating the input', () => {
    const copy = [...rows]
    expect(sortRows(rows, byN, 'asc').map((r) => r.n)).toEqual([1, 2, 3])
    expect(sortRows(rows, byN, 'desc').map((r) => r.n)).toEqual([3, 2, 1])
    expect(rows).toEqual(copy)
  })

  it('is stable for equal keys', () => {
    const tied = [
      { name: 'x', n: 1 },
      { name: 'y', n: 1 },
      { name: 'z', n: 0 },
    ]
    expect(sortRows(tied, byN, 'asc').map((r) => r.name)).toEqual(['z', 'x', 'y'])
    expect(sortRows(tied, byN, 'desc').map((r) => r.name)).toEqual(['x', 'y', 'z'])
  })
})
