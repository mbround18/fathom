import { describe, expect, it } from 'vitest'

import { blockedText, RESTRICTED_OPS, untrustedText } from '@/features/network/model'
import { BROWSER_MAX, shortBrowser, sortNetworks, sortSessions, whitelistProblem } from '@/features/sessions/model'
import type { NetworkView, SessionView } from '@/protocol/generated'

const session = (id_hint: string, last_used: string, current = false): SessionView => ({
  id_hint,
  created: '2026-10-01T00:00:00Z',
  last_used,
  address: '203.0.113.7',
  network: '203.0.113.7/32',
  trusted: true,
  location: 'unknown',
  browser: 'Firefox on Linux',
  current,
})

describe('shortBrowser', () => {
  it('keeps a short string as it is', () => {
    expect(shortBrowser('Firefox on Linux')).toBe('Firefox on Linux')
  })
  it('shortens to the limit with an ellipsis', () => {
    const out = shortBrowser('x'.repeat(200))
    expect(out).toHaveLength(BROWSER_MAX)
    expect(out.endsWith('…')).toBe(true)
  })
  it('flattens control characters and runs of space to one line', () => {
    expect(shortBrowser('Fire\nfox\t\u0000 on\u001b[31m Linux')).toBe('Fire fox on [31m Linux')
  })
  it('leaves markup as text: rendering escapes it', () => {
    expect(shortBrowser('<img src=x onerror=alert(1)>')).toBe('<img src=x onerror=alert(1)>')
  })
  it('names an empty string', () => {
    expect(shortBrowser('  \n ')).toBe('Unknown browser')
  })
})

describe('sorting', () => {
  it('puts the current session first, then the latest used', () => {
    const list = [session('a', '2026-10-02T00:00:00Z'), session('b', '2026-10-03T00:00:00Z'), session('c', '2026-10-01T00:00:00Z', true)]
    expect(sortSessions(list).map((s) => s.id_hint)).toEqual(['c', 'b', 'a'])
  })
  it('does the same for networks', () => {
    const n = (key: string, last_used: string, current = false) => ({ key, last_used, current }) as NetworkView
    expect(sortNetworks([n('x', '2026-10-01'), n('y', '2026-10-05'), n('z', '2026-09-01', true)]).map((v) => v.key)).toEqual(['z', 'y', 'x'])
  })
})

describe('whitelistProblem', () => {
  it.each(['198.51.100.7', '198.51.100.0/24', '0.0.0.0/0', '2001:db8::/48', '2001:db8::1'])('accepts %s', (v) => {
    expect(whitelistProblem(v)).toBeNull()
  })
  it.each(['', 'home', '198.51.100.256', '198.51.100.0/33', '2001:db8::/129', '1.2.3.4/24/1', '198.51.100.0/x'])('refuses %s', (v) => {
    expect(whitelistProblem(v)).not.toBeNull()
  })
})

describe('untrusted text', () => {
  it('says when a home network matures, as a date', () => {
    expect(untrustedText({ reason: 'maturing', matures_at: '2026-10-30T12:00:00Z' })).toContain('trusted from 2026-10-30')
  })
  it('says a public network stays restricted', () => {
    expect(untrustedText({ reason: 'public' })).toContain('stays restricted')
  })
  it('is null on a trusted network', () => {
    expect(blockedText({ trusted: true, reason: 'first', network: '203.0.113.7/32' })).toBeNull()
    expect(blockedText({ trusted: false, reason: 'unknown', network: '198.51.100.7/32' })).toMatch(/^Not from this network/)
  })
  it('allows start, stop and restart only', () => {
    expect([...RESTRICTED_OPS].sort()).toEqual(['restart', 'service_restart', 'service_stop', 'service_up', 'stop', 'up'])
  })
})
