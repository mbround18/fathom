import { describe, expect, it } from 'vitest'

import type { AuditLogEntry } from '@/protocol/types'

import {
  applyAuditFilter,
  DEFAULT_AUDIT_FILTER,
  hasActors,
  isDefaultFilter,
} from './auditLogFilter'

const base: AuditLogEntry = {
  id: '1',
  timestamp: '2026-03-01T10:00:00+00:00',
  action: 'delete',
  actionLabel: 'Delete',
  context: 'c',
  targetKind: 'Pod',
  targetNamespace: null,
  targetName: 'p',
  outcome: { type: 'Success' },
  requestId: null,
}
const bob: AuditLogEntry = { ...base, id: '2', actor: { sub: 's-bob', username: 'bob' } }
const alice: AuditLogEntry = { ...base, id: '3', actor: { sub: 's-alice', username: 'alice' } }

describe('audit actor filter', () => {
  it('filters by actor username', () => {
    const out = applyAuditFilter([base, bob, alice], { ...DEFAULT_AUDIT_FILTER, actor: 'bob' })
    expect(out.map((e) => e.id)).toEqual(['2'])
  })
  it('counts actor as a non-default filter', () => {
    expect(isDefaultFilter({ ...DEFAULT_AUDIT_FILTER, actor: 'bob' })).toBe(false)
    expect(isDefaultFilter(DEFAULT_AUDIT_FILTER)).toBe(true)
  })
  it('hasActors is false for desktop entries and true when any has an actor', () => {
    expect(hasActors([base])).toBe(false)
    expect(hasActors([])).toBe(false)
    expect(hasActors([base, bob])).toBe(true)
  })
})
