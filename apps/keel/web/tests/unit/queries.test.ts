import { describe, expect, it } from 'vitest'

import {
  askedText,
  outcomeText,
  outcomeTone,
  paramOf,
  paramProblem,
  paramsFor,
  upsertQuery,
  validPath,
  validUnit,
} from '@/features/hosts/queries'
import type { HostQuery } from '@/protocol/wire'

function q(over: Partial<HostQuery>): HostQuery {
  return {
    id: '01',
    host: 'h1',
    kind: 'facts',
    params: {},
    asked_by: 'ana',
    asked_at: '2026-11-17T10:00:00Z',
    outcome: null,
    answer: null,
    refusal: null,
    answered_at: null,
    ...over,
  }
}

describe('query parameters', () => {
  it('names the one parameter each kind takes', () => {
    expect(paramOf('unit_state')).toBe('unit')
    expect(paramOf('path_free')).toBe('path')
    expect(paramOf('facts')).toBeNull()
    expect(paramOf('disk_health')).toBeNull()
    expect(paramOf('applied_revision')).toBeNull()
  })

  it('checks units like the runner', () => {
    expect(validUnit('nginx.service')).toBe(true)
    expect(validUnit('postgresql@16-main.service')).toBe(true)
    expect(validUnit('')).toBe(false)
    expect(validUnit('a b')).toBe(false)
    expect(validUnit('x;reboot')).toBe(false)
    expect(validUnit('a/b')).toBe(false)
    expect(validUnit('u'.repeat(256))).toBe(true)
    expect(validUnit('u'.repeat(257))).toBe(false)
  })

  it('checks paths like the runner', () => {
    expect(validPath('/')).toBe(true)
    expect(validPath('/var/lib')).toBe(true)
    expect(validPath('var/lib')).toBe(false)
    expect(validPath('/var/lib/')).toBe(false)
    expect(validPath('/var//lib')).toBe(false)
    expect(validPath('/var/./lib')).toBe(false)
    expect(validPath('/srv/../etc/shadow')).toBe(false)
    expect(validPath('/a\0b')).toBe(false)
    expect(validPath(`/${'a'.repeat(4095)}`)).toBe(true)
    expect(validPath(`/${'a'.repeat(4096)}`)).toBe(false)
  })

  it('says why a parameter cannot be sent, and nothing for kinds without one', () => {
    expect(paramProblem('facts', '')).toBeNull()
    expect(paramProblem('unit_state', '')).toMatch(/unit/)
    expect(paramProblem('unit_state', 'a b')).not.toBeNull()
    expect(paramProblem('unit_state', 'sshd.service')).toBeNull()
    expect(paramProblem('path_free', '/srv/..')).not.toBeNull()
    expect(paramProblem('path_free', '/srv')).toBeNull()
  })

  it('sends only the kind’s parameter', () => {
    expect(paramsFor('unit_state', 'sshd.service')).toEqual({ unit: 'sshd.service' })
    expect(paramsFor('path_free', '/srv')).toEqual({ path: '/srv' })
    expect(paramsFor('facts', 'ignored')).toEqual({})
  })
})

describe('outcomes', () => {
  it('reads each outcome', () => {
    expect(outcomeText(q({}))).toBe('waiting')
    expect(outcomeText(q({ outcome: 'answered' }))).toBe('answered')
    expect(outcomeText(q({ outcome: 'refused', refusal: 'not_allowed' }))).toBe('refused: not allowed by the host’s queries.toml')
    expect(outcomeText(q({ outcome: 'refused', refusal: 'something_new' }))).toBe('refused: something_new')
    expect(outcomeText(q({ outcome: 'timeout' }))).toBe('no answer in time')
    expect(outcomeText(q({ outcome: 'no_capability' }))).toBe('runner too old for queries')
    expect(outcomeText(q({ outcome: 'disconnected' }))).toBe('host not connected')
  })

  it('tones them', () => {
    expect(outcomeTone(null)).toBe('quiet')
    expect(outcomeTone('answered')).toBe('ok')
    expect(outcomeTone('refused')).toBe('err')
    expect(outcomeTone('timeout')).toBe('warn')
  })

  it('shows what was asked with its parameter', () => {
    expect(askedText(q({ kind: 'unit_state', params: { unit: 'nginx.service' } }))).toBe('Unit state nginx.service')
    expect(askedText(q({ kind: 'disk_health' }))).toBe('Disk health')
  })
})

describe('upsertQuery', () => {
  it('keeps newest first and replaces a settled query in place', () => {
    const old = q({ id: '01', asked_at: '2026-11-17T09:00:00Z' })
    const mid = q({ id: '02', asked_at: '2026-11-17T10:00:00Z' })
    let items = upsertQuery([], old)
    items = upsertQuery(items, mid)
    expect(items.map((i) => i.id)).toEqual(['02', '01'])
    items = upsertQuery(items, { ...old, outcome: 'answered', answer: { ok: true } })
    expect(items.map((i) => i.id)).toEqual(['02', '01'])
    expect(items[1]?.outcome).toBe('answered')
    expect(items).toHaveLength(2)
  })

  it('breaks a tie on time by id, newest id first', () => {
    const a = q({ id: '0a' })
    const b = q({ id: '0b' })
    expect(upsertQuery([a], b).map((i) => i.id)).toEqual(['0b', '0a'])
  })
})
