/**
 * 016-command-palette: query parsing, candidate building, and ranking/capping for the command
 * palette (constitution III: "search ... MUST have unit tests").
 */
import { describe, expect, it } from 'vitest'

import type { ListKey, ListState } from '@/store'
import { listKey } from '@/store'
import { NAV_GROUPS } from '@/resources/nav'
import { pods } from '@/resources/pods'
import {
  buildNamespaceCandidates,
  buildResourceCandidates,
  buildSectionCandidates,
  parsePaletteQuery,
  rankAndFilter,
  type PaletteResourcePayload,
} from '@/resources/paletteIndex'

import { PODS_500 } from '../fixtures'

function listStateOf(rows: readonly unknown[]): ListState {
  return { rows: rows as ListState['rows'], status: 'ready', updatedAt: 0 }
}

describe('parsePaletteQuery', () => {
  it('treats a query with no colon as a plain name', () => {
    expect(parsePaletteQuery('my-app')).toEqual({ raw: 'my-app', kind: null, name: 'my-app' })
  })

  it('splits kind:name on the first colon', () => {
    expect(parsePaletteQuery('pod:my-app')).toEqual({
      raw: 'pod:my-app',
      kind: 'pod',
      name: 'my-app',
    })
  })

  it('allows an empty name after a trailing colon', () => {
    expect(parsePaletteQuery('pod:')).toEqual({ raw: 'pod:', kind: 'pod', name: '' })
  })

  it('splits on the first colon only, keeping later colons in the name', () => {
    expect(parsePaletteQuery('pod:a:b')).toEqual({ raw: 'pod:a:b', kind: 'pod', name: 'a:b' })
  })
})

describe('buildResourceCandidates', () => {
  const [p1, p2] = PODS_500 as [
    (typeof PODS_500)[number],
    (typeof PODS_500)[number],
    ...(typeof PODS_500)[number][],
  ]

  it('only includes rows from the active context', () => {
    const lists: Record<ListKey, ListState> = {
      [listKey('kind-a', pods.section, 'Namespaced', null)]: listStateOf([p1]),
      [listKey('kind-b', pods.section, 'Namespaced', null)]: listStateOf([p2]),
    }
    const results = buildResourceCandidates(lists, 'kind-a')
    expect(results).toHaveLength(1)
    expect(results[0]?.label).toBe(p1.name)
    expect((results[0]?.payload as PaletteResourcePayload).ref.name).toBe(p1.name)
  })

  it('skips list keys for sections with no registered descriptor', () => {
    const lists: Record<ListKey, ListState> = {
      [listKey('kind-a', 'not-a-real-section', 'Namespaced', null)]: listStateOf([p1]),
    }
    expect(buildResourceCandidates(lists, 'kind-a')).toHaveLength(0)
  })
})

describe('buildNamespaceCandidates', () => {
  it('maps namespace names to results, and returns empty when none are loaded', () => {
    expect(buildNamespaceCandidates([])).toHaveLength(0)
    const results = buildNamespaceCandidates(['default', 'payments'])
    expect(results.map((r) => r.label)).toEqual(['default', 'payments'])
    expect(results.every((r) => r.category === 'namespace')).toBe(true)
  })
})

describe('buildSectionCandidates', () => {
  it('returns a non-empty, static set with no store dependency', () => {
    const results = buildSectionCandidates()
    expect(results.length).toBeGreaterThan(0)
    expect(results.every((r) => r.category === 'section')).toBe(true)
    expect(results.some((r) => r.label === 'Cluster settings')).toBe(true)
  })

  it('indexes "More" (advanced) kinds under their new groups', () => {
    const results = buildSectionCandidates()
    const find = (label: string) => results.find((r) => r.label === label)
    expect(find('ReplicaSets')?.sublabel).toBe('Workloads')
    expect(find('Endpoints')?.sublabel).toBe('Networking')
    expect(find('Leases')?.sublabel).toBe('Configuration')
    expect(find('Horizontal Pod Autoscalers')?.sublabel).toBe('Scaling')
    expect(find('Cluster settings')?.sublabel).toBe('Settings')
  })

  it('covers every section the navigator renders, once', () => {
    const ids = buildSectionCandidates().map((r) => r.id)
    const rendered = NAV_GROUPS.flatMap((g) => g.sections.map((s) => `section:${s.key}`))
    expect(ids).toEqual(rendered)
    expect(new Set(ids).size).toBe(ids.length)
  })
})

describe('rankAndFilter', () => {
  const [p1] = PODS_500 as [(typeof PODS_500)[number], ...(typeof PODS_500)[number][]]
  const lists: Record<ListKey, ListState> = {
    [listKey('kind-a', pods.section, 'Namespaced', null)]: listStateOf(PODS_500),
  }
  const resources = buildResourceCandidates(lists, 'kind-a')

  it('filters resources by a fuzzily-matched kind prefix', () => {
    const query = parsePaletteQuery(`pod:${p1.name}`)
    const results = rankAndFilter(resources, query, 50)
    expect(results.some((r) => r.label === p1.name)).toBe(true)
  })

  it('yields zero resource results for an unrecognized kind prefix', () => {
    const query = parsePaletteQuery(`xyz:${p1.name}`)
    const results = rankAndFilter(resources, query, 50)
    expect(results).toHaveLength(0)
  })

  it('ranks stronger matches first', () => {
    const query = parsePaletteQuery(p1.name)
    const results = rankAndFilter(resources, query, 50)
    const top = results[0]
    expect(top?.label).toBe(p1.name)
  })

  it('never returns more than `limit` results per category', () => {
    const query = parsePaletteQuery('')
    const results = rankAndFilter(resources, query, 3)
    expect(results.length).toBeLessThanOrEqual(3)
  })
})
