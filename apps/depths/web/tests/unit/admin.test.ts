import { describe, expect, it } from 'vitest'

import { cleanFilter, detailLines, exportHref, isoToLocal, localToIso, outcomeText } from '@/features/audit/model'
import { isUserId, missingStacks, sortUsers, stackChoices } from '@/features/users/model'
import type { AuditEntry, User } from '@/protocol/generated'

const user = (id: string, over: Partial<User> = {}): User => ({
  id,
  provider: 'github',
  issuer: null,
  subject: id,
  username: id,
  email: null,
  role: 'user',
  stacks: [],
  owner: false,
  pinned_from: null,
  first_seen: null,
  last_seen: null,
  ...over,
})

describe('users model', () => {
  it('accepts provider ids only', () => {
    expect(isUserId('discord:123')).toBe(true)
    expect(isUserId('github:42')).toBe(true)
    expect(isUserId('oidc:a|b')).toBe(true)
    for (const bad of ['ada', 'github:', 'github: 42', 'gitlab:1', ' github:1', 'github::1']) expect(isUserId(bad)).toBe(false)
  })

  it('marks assigned stacks with no directory', () => {
    expect(missingStacks({ stacks: ['a', 'gone', 'b'] }, new Set(['a', 'b']))).toEqual(['gone'])
  })

  it('offers the stacks on disk, then the missing assigned ones', () => {
    expect(stackChoices(['b', 'a'], ['gone', 'a'])).toEqual(['a', 'b', 'gone'])
  })

  it('sorts owners, then admins, then by name', () => {
    const sorted = sortUsers([user('zed'), user('amy'), user('bo', { role: 'admin' }), user('own', { owner: true, role: 'admin' })])
    expect(sorted.map((u) => u.id)).toEqual(['own', 'bo', 'amy', 'zed'])
  })
})

describe('audit model', () => {
  it('builds a same-origin export path from the filter', () => {
    expect(exportHref({})).toBe('/api/v1/audit/export')
    expect(exportHref({ stack: 'web', action: 'up', from: '2026-10-20T09:00:00.000Z' })).toBe(
      '/api/v1/audit/export?stack=web&action=up&from=2026-10-20T09%3A00%3A00.000Z',
    )
    // Values are query-encoded: no way to smuggle another parameter in.
    expect(exportHref({ actor: 'oidc:a&to=x' })).toBe('/api/v1/audit/export?actor=oidc%3Aa%26to%3Dx')
  })

  it('drops empty filter fields', () => {
    expect(cleanFilter({ stack: '', actor: 'github:1' })).toEqual({ actor: 'github:1' })
  })

  it('round-trips local times', () => {
    const iso = localToIso('2026-10-20T09:30')
    expect(iso).toMatch(/Z$/)
    expect(isoToLocal(iso)).toBe('2026-10-20T09:30')
    expect(localToIso('')).toBeUndefined()
    expect(localToIso('nonsense')).toBeUndefined()
  })

  it('describes outcomes and details as text', () => {
    expect(outcomeText({ kind: 'failed', exit_code: 1, stderr_tail: '' })).toBe('Failed (exit 1)')
    expect(outcomeText({ kind: 'denied', reason: 'not_permitted' })).toBe('Denied: not_permitted')
    const e: AuditEntry = {
      at: '2026-10-20T09:00:00Z',
      actor: { provider: 'github', subject: '1', username: 'ada' },
      action: 'user_changed',
      stack: null,
      preview_id: null,
      operation_id: null,
      argv: [['docker', 'compose', 'up', 'a b']],
      target: { id: 'github:2', before: { role: 'user', stacks: [] }, after: { role: 'admin', stacks: [] } },
      diff_summary: null,
      outcome: { kind: 'succeeded' },
      source_ip: '10.0.0.1',
    }
    expect(detailLines(e)).toEqual(["docker compose up 'a b'", 'github:2: user on no stacks → admin'])
  })
})
