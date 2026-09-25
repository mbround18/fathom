import { describe, expect, it } from 'vitest'

import { deriveFocus, NAMESPACE_FOCUS_SCALE, WORKLOAD_FOCUS_SCALE } from '@/topology/viewport'

describe('deriveFocus', () => {
  const namespaceNodes = [{ label: 'default' }]
  const workloadNodes = [{ kind: 'Deployment' as const, label: 'web', parentId: 'ns-1' }]
  const namespaceIdToName = new Map([['ns-1', 'default']])

  it('has no focus below the namespace threshold', () => {
    const r = deriveFocus({
      scale: NAMESPACE_FOCUS_SCALE - 0.1,
      namespaceNodes,
      workloadNodes,
      namespaceIdToName,
    })
    expect(r.namespaceFocus.size).toBe(0)
    expect(r.workloadFocus.size).toBe(0)
  })

  it('expands namespaces once the namespace threshold is crossed, workloads stay collapsed', () => {
    const r = deriveFocus({
      scale: NAMESPACE_FOCUS_SCALE,
      namespaceNodes,
      workloadNodes,
      namespaceIdToName,
    })
    expect(r.namespaceFocus.has('default')).toBe(true)
    expect(r.workloadFocus.size).toBe(0)
  })

  it('expands workloads once the workload threshold is crossed (US2 acceptance scenario 2)', () => {
    const r = deriveFocus({
      scale: WORKLOAD_FOCUS_SCALE,
      namespaceNodes,
      workloadNodes,
      namespaceIdToName,
    })
    expect(r.workloadFocus.has('default/Deployment/web')).toBe(true)
  })

  it('recomputes fresh each call, no persisted state (US2 acceptance scenario 3)', () => {
    const zoomedIn = deriveFocus({
      scale: WORKLOAD_FOCUS_SCALE,
      namespaceNodes,
      workloadNodes,
      namespaceIdToName,
    })
    expect(zoomedIn.workloadFocus.size).toBe(1)
    const zoomedOut = deriveFocus({ scale: 1, namespaceNodes, workloadNodes, namespaceIdToName })
    expect(zoomedOut.workloadFocus.size).toBe(0)
    expect(zoomedOut.namespaceFocus.size).toBe(0)
  })
})
