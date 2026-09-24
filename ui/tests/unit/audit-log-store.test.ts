import { beforeEach, describe, expect, it } from 'vitest'

import type { AuditLogEntry } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { ingest } from '@/store/ingest'

import { batch, system } from '../fixtures/builders'

function entry(id: string): AuditLogEntry {
  return {
    id,
    timestamp: '2026-01-01T00:00:00Z',
    action: 'scale',
    actionLabel: 'Scale',
    context: 'a',
    targetKind: 'Deployment',
    targetNamespace: 'default',
    targetName: id,
    outcome: { type: 'Success' },
    requestId: null,
  }
}

describe('AuditLogLoaded handler', () => {
  beforeEach(() => {
    useStore.setState(initialData())
  })

  it('stores entries newest first and replaces them on refresh', () => {
    ingest(
      batch([system(1, { type: 'AuditLogLoaded', data: { entries: [entry('1'), entry('2')] } })]),
    )
    expect(useStore.getState().audit.entries.map((e) => e.id)).toEqual(['2', '1'])
    expect(useStore.getState().audit.loaded).toBe(true)
    ingest(
      batch([
        system(2, {
          type: 'AuditLogLoaded',
          data: { entries: [entry('1'), entry('2'), entry('3')] },
        }),
      ]),
    )
    expect(useStore.getState().audit.entries.map((e) => e.id)).toEqual(['3', '2', '1'])
  })
})
