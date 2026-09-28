/** 043 T006: the inspector Topology tab's upstream/downstream split (FR-031). */
import { describe, expect, it } from 'vitest'

import type { RelatedResourceRef } from '@/protocol/types'
import { RESOLVER_KINDS, UPSTREAM, relationLabel, splitRelated } from '@/topology/resource-topology'

const ref = (
  kind: string,
  name: string,
  relation: string,
  over: Partial<RelatedResourceRef> = {},
): RelatedResourceRef => ({
  section: `workloads/${kind.toLowerCase()}s`,
  kind,
  namespace: 'shop',
  name,
  relation,
  status_hint: null,
  ...over,
})

describe('splitRelated', () => {
  it('returns empty sides for a root with no neighbours', () => {
    const root = { kind: 'Pod', namespace: 'shop', name: 'lonely' }
    expect(splitRelated(root, [])).toEqual({ root, upstream: [], downstream: [] })
  })

  it('splits a Pod: owner and Service/PVC membership up, its node down', () => {
    const root = { kind: 'Pod', namespace: 'shop', name: 'web-7c9-abc' }
    const t = splitRelated(root, [
      ref('ReplicaSet', 'web-7c9', 'owned-by'),
      ref('Service', 'web', 'member-of', { section: 'network/services' }),
      ref('Node', 'node-1', 'scheduled-on', { section: 'nodes', namespace: null }),
      ref('ScaledObject', 'web', 'scaledBy'),
    ])
    expect(t.upstream.map((n) => [n.kind, n.name, n.relationLabel])).toEqual([
      ['ReplicaSet', 'web-7c9', 'owned by'],
      ['Service', 'web', 'member of'],
      ['ScaledObject', 'web', 'scaled by'],
    ])
    expect(t.downstream.map((n) => [n.kind, n.name, n.relationLabel])).toEqual([
      ['Node', 'node-1', 'scheduled on'],
    ])
  })

  it('puts a Service’s selected Pods downstream', () => {
    const root = { kind: 'Service', namespace: 'shop', name: 'web' }
    const t = splitRelated(root, [
      ref('Pod', 'web-1', 'selects', { status_hint: 'Running' }),
      ref('Pod', 'web-2', 'selects', { status_hint: 'Pending' }),
    ])
    expect(t.upstream).toEqual([])
    expect(t.downstream.map((n) => [n.name, n.status_hint, n.relationLabel])).toEqual([
      ['web-1', 'Running', 'selects'],
      ['web-2', 'Pending', 'selects'],
    ])
  })

  it('puts the Pods mounting a PVC upstream', () => {
    const root = { kind: 'PersistentVolumeClaim', namespace: 'shop', name: 'data' }
    const t = splitRelated(root, [ref('Pod', 'db-0', 'mounted-by')])
    expect(t.upstream.map((n) => [n.name, n.relationLabel])).toEqual([['db-0', 'mounted by']])
    expect(t.downstream).toEqual([])
  })

  it('puts owned objects, scale targets and a Flux reconciler on the right sides', () => {
    const root = { kind: 'HelmRelease', namespace: 'shop', name: 'web' }
    const t = splitRelated(root, [
      ref('Deployment', 'web', 'owns'),
      ref('HelmRelease', 'web', 'reconciled-by', { section: 'integrations/flux' }),
      ref('Deployment', 'worker', 'scales'),
    ])
    expect(t.upstream.map((n) => n.relation)).toEqual(['reconciled-by'])
    expect(t.downstream.map((n) => n.relation)).toEqual(['owns', 'scales'])
  })

  it('puts an unknown relation downstream, labelled with its own name', () => {
    const root = { kind: 'Deployment', namespace: 'shop', name: 'web' }
    const t = splitRelated(root, [ref('Gateway', 'edge', 'routes-to')])
    expect(t.upstream).toEqual([])
    expect(t.downstream).toHaveLength(1)
    expect(t.downstream[0]).toMatchObject({ kind: 'Gateway', relationLabel: 'routes-to' })
  })

  it('keeps every ref field so Follow can open the neighbour', () => {
    const r = ref('ReplicaSet', 'web-7c9', 'owned-by')
    const [n] = splitRelated({ kind: 'Pod', namespace: 'shop', name: 'p' }, [r]).upstream
    expect(n).toEqual({ ...r, relationLabel: 'owned by' })
  })
})

describe('relation sets', () => {
  it('lists the upstream relations from data-model.md', () => {
    expect([...UPSTREAM].sort()).toEqual(
      ['member-of', 'mounted-by', 'owned-by', 'reconciled-by', 'scaledBy'].sort(),
    )
  })

  it('labels known relations in plain words and leaves unknown ones as they are', () => {
    expect(relationLabel('scaledBy')).toBe('scaled by')
    expect(relationLabel('custom-link')).toBe('custom-link')
  })

  it('offers the tab only for resolver kinds', () => {
    for (const k of ['Node', 'Service', 'PersistentVolumeClaim', 'Pod', 'Deployment'])
      expect(RESOLVER_KINDS.has(k), k).toBe(true)
    for (const k of ['StatefulSet', 'DaemonSet', 'ReplicaSet', 'ScaledObject', 'ScaledJob'])
      expect(RESOLVER_KINDS.has(k), k).toBe(true)
    for (const k of ['Application', 'HelmRelease']) expect(RESOLVER_KINDS.has(k), k).toBe(true)
    expect(RESOLVER_KINDS.has('ConfigMap')).toBe(false)
    expect(RESOLVER_KINDS.has('Secret')).toBe(false)
    expect(RESOLVER_KINDS.size).toBe(12)
  })
})
