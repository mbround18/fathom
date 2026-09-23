import { describe, expect, it } from 'vitest'

import { nodeSummaryTone, podTone, workloadTone } from '@/topology/status'

describe('topology status tone', () => {
  it('colors a running pod ok, a failed pod err', () => {
    expect(podTone({ phase: 'Running' })).toBe('ok')
    expect(podTone({ phase: 'Failed' })).toBe('err')
  })

  it('colors a fully-ready workload ok, an under-ready one warn', () => {
    expect(workloadTone({ name: 'w', namespace: 'default', desired: 3, ready: 3, updated: 3, available: 3 })).toBe('ok')
    expect(workloadTone({ name: 'w', namespace: 'default', desired: 3, ready: 1, updated: 1, available: 1 })).toBe('warn')
  })

  it('colors a ready node ok', () => {
    expect(
      nodeSummaryTone({
        name: 'node-1',
        status: 'Ready',
        role: 'worker',
        pod_count: 1,
        pod_capacity: 10,
        cpu_allocatable_millicores: 1000,
        memory_allocatable_bytes: 1,
      }),
    ).toBe('ok')
  })

  // 007-topology-graph-view US4/T050: `nodeSummaryTone` is a pure function of the row, so a
  // live status change recomputes tone with no new production code — this is the regression
  // coverage for that (the e2e side, `us4-topology-live-update.spec.ts`, only proves the
  // redraw-in-place wiring, since pixel-exact color isn't reliably sampled across browsers there).
  it('recomputes tone when a node goes from ready to not-ready', () => {
    const base = {
      name: 'node-1',
      role: 'worker',
      pod_count: 1,
      pod_capacity: 10,
      cpu_allocatable_millicores: 1000,
      memory_allocatable_bytes: 1,
    }
    expect(nodeSummaryTone({ ...base, status: 'Ready' })).toBe('ok')
    expect(nodeSummaryTone({ ...base, status: 'NotReady' })).not.toBe('ok')
  })
})
