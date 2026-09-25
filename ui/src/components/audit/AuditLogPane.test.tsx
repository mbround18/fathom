import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { AuditLogEntry } from '@/protocol/types'
import { useStore } from '@/store'
import { DEFAULT_AUDIT_FILTER } from '@/store/auditLogFilter'

import { AuditLogPane } from './AuditLogPane'

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

function load(entries: AuditLogEntry[]) {
  useStore.setState({
    audit: { entries, loaded: true, visible: false, filter: DEFAULT_AUDIT_FILTER },
    refreshAuditLog: vi.fn(),
  } as never)
}

describe('AuditLogPane actor column', () => {
  beforeEach(() => {
    load([])
  })
  it('is hidden when no entry carries an actor', () => {
    load([base])
    render(<AuditLogPane />)
    expect(screen.queryByTestId('audit-actor')).toBeNull()
    expect(screen.queryByLabelText('Actor')).toBeNull()
  })
  it('shows the column and filter when an entry has an actor', () => {
    load([base, { ...base, id: '2', actor: { sub: 's', username: 'bob' } }])
    render(<AuditLogPane />)
    expect(screen.getAllByTestId('audit-actor')).toHaveLength(2)
    expect(screen.getByLabelText('Actor')).toBeTruthy()
  })
})
