import { describe, expect, it } from 'vitest'

import { groupHosts, hasZones, pathText, reachDetail, reachText, reachTone, streams, zoneName } from '@/features/hosts/reach'
import type { HostInfo, RelayInfo, ZoneInfo } from '@/protocol/wire'

const host = (over: Partial<HostInfo> = {}): HostInfo => ({
  id: 'h-1',
  workspace: 'w-1',
  inventory_name: 'web-1',
  state: 'offline',
  profiles: ['base'],
  allowed_profiles: ['base'],
  require_signed: false,
  runner_version: null,
  ansible_version: null,
  enrolled_at: '2026-10-27T12:00:00Z',
  last_seen_at: null,
  revoked_at: null,
  changed_outside: false,
  age_recipient: null,
  inventory_status: 'unknown',
  zone: null,
  reach: { kind: 'disconnected' },
  converge: null,
  suggestions: 0,
  ...over,
})

const zone = (id: string, name: string): ZoneInfo => ({ id, workspace: 'w-1', name, description: '', hosts: 0, relays: 0 })

const relay = (over: Partial<RelayInfo> = {}): RelayInfo => ({
  id: 'r-1',
  workspace: 'w-1',
  zone: 'z-lab',
  name: 'gw-lab',
  dial_address: 'gw.lab:7444',
  host_key_fingerprint: 'SHA256:abc',
  host: null,
  state: 'connected',
  since: null,
  rtt_ms: 12,
  version: '0.1.0',
  streams: 2,
  last_error: null,
  revoked_at: null,
  ...over,
})

const RELAYS = [relay(), relay({ id: 'r-2', name: 'gw-old', state: 'revoked' })]

describe('reach text (data-model.md "Host (display, derived)")', () => {
  it('words every row of the table', () => {
    expect(reachText({ kind: 'connected', path: { kind: 'direct' } }, RELAYS)).toBe('connected · direct')
    expect(reachText({ kind: 'connected', path: { kind: 'relay', relay: 'r-1' } }, RELAYS)).toBe('connected · via gw-lab')
    expect(reachText({ kind: 'disconnected' }, RELAYS)).toBe('disconnected')
    expect(reachText({ kind: 'relay_unreachable', relays: ['r-1'] }, RELAYS)).toBe('disconnected (relay unreachable)')
    expect(reachText({ kind: 'relay_revoked', relay: 'r-2' }, RELAYS)).toBe('disconnected (relay revoked)')
    expect(reachText({ kind: 'no_path' }, RELAYS)).toBe('no path')
  })

  it('the Path column says only how, while connected', () => {
    expect(pathText({ kind: 'connected', path: { kind: 'direct' } }, RELAYS)).toBe('direct')
    expect(pathText({ kind: 'connected', path: { kind: 'relay', relay: 'r-1' } }, RELAYS)).toBe('via gw-lab')
    // A relay the list has not seen yet is still a relay.
    expect(pathText({ kind: 'connected', path: { kind: 'relay', relay: 'r-9' } }, RELAYS)).toBe('via a relay')
  })

  it('names the relays behind an unreachable or revoked path', () => {
    expect(reachDetail({ kind: 'relay_unreachable', relays: ['r-1'] }, RELAYS)).toContain('gw-lab')
    expect(reachDetail({ kind: 'relay_revoked', relay: 'r-2' }, RELAYS)).toContain('gw-old')
    expect(reachDetail({ kind: 'disconnected' }, RELAYS)).toBeNull()
  })

  it('tones: ok, quiet, warn for a down relay, err with no way in', () => {
    expect(reachTone({ kind: 'connected', path: { kind: 'direct' } })).toBe('ok')
    expect(reachTone({ kind: 'disconnected' })).toBe('quiet')
    expect(reachTone({ kind: 'relay_unreachable', relays: [] })).toBe('warn')
    expect(reachTone({ kind: 'no_path' })).toBe('err')
  })
})

describe('grouping by zone', () => {
  const zones = [zone('z-lab', 'lab'), zone('z-dmz', 'dmz')]
  const hosts = [
    host({ id: 'a', inventory_name: 'a', zone: 'z-lab' }),
    host({ id: 'b', inventory_name: 'b' }),
    host({ id: 'c', inventory_name: 'c', zone: 'z-gone' }),
  ]

  it('puts direct first, then zones by name, then hosts in a zone not listed yet', () => {
    const groups = groupHosts(hosts, zones, [relay({ state: 'unreachable' }), relay({ id: 'r-3', name: 'gw-b' })])
    expect(groups.map((g) => g.name)).toEqual(['direct', 'dmz', 'lab', 'unknown zone'])
    expect(groups[0]?.hosts.map((h) => h.id)).toEqual(['b'])
    expect(groups[1]?.hosts).toEqual([])
    // Working relays first.
    expect(groups[2]?.relays.map((r) => r.name)).toEqual(['gw-b', 'gw-lab'])
  })

  it('under a filter, only zones with a matching host show', () => {
    expect(groupHosts([hosts[0] as HostInfo], zones, [], true).map((g) => g.name)).toEqual(['lab'])
  })

  it('without zones, everything is direct, and the page does not group', () => {
    const plain = [host()]
    expect(hasZones([], plain)).toBe(false)
    expect(hasZones(zones, plain)).toBe(true)
    expect(hasZones([], [host({ zone: 'z-lab' })])).toBe(true)
    expect(groupHosts(plain, [], []).map((g) => g.name)).toEqual(['direct'])
  })

  it('names zones and counts streams', () => {
    expect(zoneName(null, zones)).toBe('direct')
    expect(zoneName('z-dmz', zones)).toBe('dmz')
    expect(streams(relay({ streams: 1 }))).toBe('1 stream')
    expect(streams(relay({ streams: null }))).toBeNull()
  })
})
