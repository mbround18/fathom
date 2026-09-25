import { describe, expect, it } from 'vitest'

import { evaluate } from '@/linter/evaluate'
import { detailTargets, nextBatch, REQUEST_EXPIRY_MS } from '@/linter/fetchPlan'
import { filterFindings } from '@/linter/filter'
import { input, must } from './helpers'

const pods = (n: number) => Array.from({ length: n }, (_, i) => ({ name: `p${i}`, namespace: 'd' }))

describe('evaluate', () => {
  it('reports a clean empty state with a null score when nothing was checked', () => {
    const r = evaluate(input({}))
    expect(r.findings).toEqual([])
    expect(r.score).toBeNull()
    expect(r.rules.every((x) => x.status === 'skipped')).toBe(true)
  })
  it('scores, counts by severity, and sorts high first', () => {
    const r = evaluate(
      input({
        lists: { 'workloads/pods': { rows: pods(2) } },
        details: {
          'Pod|d|p0': { raw: { metadata: {}, spec: { containers: [] } } },
          'Pod|d|p1': { raw: { metadata: { ownerReferences: [{}] } } },
        },
      }),
    )
    expect(r.counts.high).toBe(1)
    expect(must(r.findings[0]).rule).toBe('naked-pod')
    expect(r.score).toBeLessThan(100)
    expect(filterFindings(r.findings, { severity: 'low', category: null })).toHaveLength(0)
    expect(
      filterFindings(r.findings, { severity: null, category: 'naked-pod' }).length,
    ).toBeGreaterThan(0)
  })
  it('never throws on malformed raw_json', () => {
    const bad = {
      rows: () => ({ status: 'ready' as const, rows: pods(1) }),
      detail: () => ({ data: { raw_json: '{nope' } }),
    }
    expect(() => evaluate(bad)).not.toThrow()
  })
})

describe('fetch plan', () => {
  it('targets active-scope objects and throttles in-flight reads', () => {
    const inp = input({
      lists: {
        'workloads/pods': { rows: pods(20) },
        'workloads/replicasets': { rows: [{ name: 'old', namespace: 'd', desired: 0 }] },
      },
    })
    const targets = detailTargets(inp)
    expect(targets.every((t) => t.kind === 'Pod')).toBe(true)
    const sent = new Map<string, number>()
    const first = nextBatch(targets, () => false, sent, 0)
    expect(first).toHaveLength(8)
    for (const t of first) sent.set(`${t.kind}|${t.namespace}|${t.name}`, 0)
    expect(nextBatch(targets, () => false, sent, 1)).toHaveLength(0)
    const arrived = new Set(first.slice(0, 3).map((t) => t.name))
    expect(nextBatch(targets, (t) => arrived.has(t.name), sent, 1)).toHaveLength(3)
    // expired requests free their slot and are not resent
    const later = nextBatch(targets, () => false, sent, REQUEST_EXPIRY_MS + 1)
    expect(later).toHaveLength(8)
    expect(later.some((t) => sent.has(`${t.kind}|${t.namespace}|${t.name}`))).toBe(false)
  })
})
