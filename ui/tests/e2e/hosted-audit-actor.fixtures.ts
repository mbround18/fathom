import type { AuditLogEntry } from '../../src/protocol/types'

export const PLAIN_ENTRY: AuditLogEntry = {
  id: 'h1',
  timestamp: '2026-03-01T10:00:00+00:00',
  action: 'delete',
  actionLabel: 'Delete',
  context: 'kind-a',
  targetKind: 'Pod',
  targetNamespace: 'default',
  targetName: 'web-1',
  outcome: { type: 'Success' },
  requestId: null,
}

export const ACTOR_ENTRIES: AuditLogEntry[] = [
  { ...PLAIN_ENTRY, id: 'h2', actor: { sub: 'sub-alice', username: 'alice' } },
  {
    ...PLAIN_ENTRY,
    id: 'h3',
    timestamp: '2026-03-01T11:00:00+00:00',
    actor: { sub: 'sub-bob', username: 'bob' },
  },
]
