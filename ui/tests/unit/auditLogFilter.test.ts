import { describe, expect, it } from 'vitest'

import type { AuditLogEntry } from '@/protocol/types'
import { applyAuditFilter, DEFAULT_AUDIT_FILTER, isDefaultFilter } from '@/store/auditLogFilter'

function e(over: Partial<AuditLogEntry>): AuditLogEntry {
  return {
    id: 'x',
    timestamp: '2026-01-02T00:00:00Z',
    action: 'nodeCordon',
    actionLabel: 'Node Cordon',
    context: 'a',
    targetKind: 'Node',
    targetNamespace: null,
    targetName: 'Worker-1',
    outcome: { type: 'Success' },
    requestId: null,
    ...over,
  }
}

const A = e({ id: '1' })
const B = e({
  id: '2',
  action: 'delete',
  context: 'b',
  targetName: 'api',
  timestamp: '2026-02-01T00:00:00Z',
  outcome: { type: 'Failure', data: { reason: 'nope' } },
})
const all = [A, B]
const f = DEFAULT_AUDIT_FILTER

describe('applyAuditFilter', () => {
  it('default filter keeps everything', () => {
    expect(isDefaultFilter(f)).toBe(true)
    expect(applyAuditFilter(all, f)).toEqual(all)
  })
  it('filters by action kind, outcome, context, time, and text', () => {
    expect(applyAuditFilter(all, { ...f, actionKinds: ['delete'] })).toEqual([B])
    expect(applyAuditFilter(all, { ...f, outcome: 'failure' })).toEqual([B])
    expect(applyAuditFilter(all, { ...f, outcome: 'success' })).toEqual([A])
    expect(applyAuditFilter(all, { ...f, context: 'a' })).toEqual([A])
    const feb = Date.parse('2026-01-15T00:00:00Z')
    expect(applyAuditFilter(all, { ...f, timeRange: { from: feb } })).toEqual([B])
    expect(applyAuditFilter(all, { ...f, timeRange: { to: feb } })).toEqual([A])
    expect(applyAuditFilter(all, { ...f, searchText: 'WORKER' })).toEqual([A])
  })
  it('combines filters and clearing restores the full list', () => {
    expect(applyAuditFilter(all, { ...f, context: 'a', outcome: 'failure' })).toEqual([])
    expect(applyAuditFilter(all, DEFAULT_AUDIT_FILTER)).toEqual(all)
  })
})
