/**
 * Shared scene for spec 067's Hosts and Settings → Relays specs (T036): workspace `ops` (spec 066's
 * set-up, catalog synced) with
 *
 * - `gate-1`, direct, which runs relay `gw-lab`;
 * - `lab-1` and `lab-2` in zone `lab`, reached through `gw-lab`;
 * - `dmz-1` in zone `dmz`, reached through `gw-dmz`.
 *
 * Every host has a runner, so it is online while its zone gives it a path. Not a spec.
 */
import type { FakeBackend } from '../../support/fake-backend'
import { hostInfo } from '../../support/fake-hosts'
import { FakeRelays } from '../../support/fake-relays'
import { setUp, type Options, type Repo } from '../s066/repo-setup'

export interface Scene extends Repo {
  relays: FakeRelays
}

export function scene(backend: FakeBackend, o: Options = {}): Scene {
  const repo = setUp(backend, o)
  const ws = repo.ops.id
  const host = (n: number, name: string) =>
    hostInfo({
      id: `host-${String(n).padStart(4, '0')}`,
      workspace: ws,
      inventory_name: name,
      state: 'online',
      runner_version: '0.1.0',
      ansible_version: '2.21.4',
      last_seen_at: '2026-10-27T12:00:00Z',
    })
  repo.hosts.hosts = [host(1, 'dmz-1'), host(2, 'gate-1'), host(3, 'lab-1'), host(4, 'lab-2')]
  const relays = new FakeRelays(backend, repo.hosts, ws)
  relays.zone('lab', 'The lab network behind gate-1.')
  relays.zone('dmz')
  relays.relay('gw-lab', 'lab', { host: 'host-0002', streams: 2, rtt_ms: 9 })
  relays.relay('gw-dmz', 'dmz', { streams: 1, rtt_ms: 31 })
  relays.place('lab-1', 'lab')
  relays.place('lab-2', 'lab')
  relays.place('dmz-1', 'dmz')
  relays.place('gate-1', null)
  return { ...repo, relays }
}
