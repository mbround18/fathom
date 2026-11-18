/**
 * Zones and relays on the fake backend (spec 067 US3, T033; contracts/protocol.md "Zones and
 * relays"). An in-memory model answering as apps/keel/server/src/zones.rs does, installed with
 * `on(...)` beside a `FakeHosts`, whose hosts it places in zones and whose reach it derives:
 *
 * - `ListZones` → `Zones`, `ListRelays` → `Relays` (the workspace's), `GetRelayLinkKeys` →
 *   `RelayLinkKeys` for instance admins, `Denied` for anyone else.
 * - Zone create / update / remove, `PreviewHostZone`, relay register / update / revoke / release
 *   and the link-key rotate / activate / retire preview with the server's summaries, warnings and
 *   step-up flags. A zone in use previews its blockers, and its confirm is refused `zone_in_use`.
 * - `Confirm` (chained) answers `Confirmed` and the new list (`Zones`, `Relays`, `RelayLinkKeys`,
 *   or `Host` for a zone move). Step-up previews need a fresh check, as the shared fake does.
 *   Relay changes also push `RelayChanged`, and every host whose reach moved `HostReachChanged`.
 * - `PreviewRunStart` previews a run over the workspace's live hosts and warns, as the server
 *   does, when one of them runs the only relay of a zone with other hosts in the run.
 *
 * Scripting: `zone(name)`, `relay(name, zone, over)` and `place(host, zone)` set the scene;
 * `setState(relay, state)` stands in for the link coming up or dropping, and re-derives the reach
 * of the zone's hosts (connected through a working relay, else the server's `derive_reach`).
 */
import type { Cmd, Event, HostInfo, HostReach, Preview, RelayInfo, RelayLinkKey, RelayState, ZoneInfo } from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import type { FakeHosts } from './fake-hosts'
import { NOW, WORKSPACE } from './fake-repos'

const EXPIRES = '2026-10-27T12:15:00Z'
export const LINK_KEY_FP = 'SHA256:keelrelaylinkkey0000000000000000000000000000'
export const LINK_KEY_LINE = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKeelRelayLink keel-relay-link'
/** A relay host key the specs paste; its fingerprint is `fingerprintOf(RELAY_KEY)`. */
export const RELAY_KEY = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIRelayHostKeyGwLab0000000000000000 root@gw-lab'

/** A stand-in for the SHA-256 fingerprint: stable for a key, and plainly a fingerprint. */
export function fingerprintOf(line: string): string {
  const body = line.trim().split(/\s+/)[1] ?? ''
  return `SHA256:${body.slice(-32).padStart(43, 'f')}`
}

type Pending =
  | { kind: 'zone-create'; name: string; description: string }
  | { kind: 'zone-update'; zone: string; name: string; description: string }
  | { kind: 'zone-remove'; zone: string; blocked: boolean }
  | { kind: 'host-zone'; host: string; zone: string | null }
  | { kind: 'register'; relay: RelayInfo }
  | { kind: 'update'; relay: RelayInfo }
  | { kind: 'revoke'; relay: string }
  | { kind: 'release'; relay: string }
  | { kind: 'rotate' }
  | { kind: 'activate'; fingerprint: string }
  | { kind: 'retire'; fingerprint: string }
  | { kind: 'run' }

const STEP_UP: ReadonlySet<Pending['kind']> = new Set(['register', 'update', 'release', 'rotate', 'activate', 'retire'])

export class FakeRelays {
  zones: ZoneInfo[] = []
  relays: RelayInfo[] = []
  keys: RelayLinkKey[] = [{ fingerprint: LINK_KEY_FP, public_line: LINK_KEY_LINE, state: 'active' }]
  private readonly pending = new Map<string, Pending>()
  private seq = 0
  private readonly backend: FakeBackend
  private readonly hosts: FakeHosts

  constructor(backend: FakeBackend, hosts: FakeHosts) {
    this.backend = backend
    this.hosts = hosts
    const types = [
      'ListZones',
      'PreviewZoneCreate',
      'PreviewZoneUpdate',
      'PreviewZoneRemove',
      'PreviewHostZone',
      'ListRelays',
      'PreviewRelayRegister',
      'PreviewRelayUpdate',
      'PreviewRelayRevoke',
      'PreviewRelayRelease',
      'GetRelayLinkKeys',
      'PreviewRelayLinkKeyRotate',
      'PreviewRelayLinkKeyActivate',
      'PreviewRelayLinkKeyRetire',
      'PreviewRunStart',
    ] as const
    for (const type of types) backend.on(type, (cmd) => this.respond(cmd))
    backend.chain('Confirm', (cmd) => this.respond(cmd))
  }

  // --- Scripting ------------------------------------------------------------------------------

  zone(name: string, description = ''): ZoneInfo {
    const z: ZoneInfo = { id: `zone-${name}`, workspace: WORKSPACE, name, description, hosts: 0, relays: 0 }
    this.zones = [...this.zones, z]
    return this.counted(z)
  }

  relay(name: string, zone: string, over: Partial<RelayInfo> = {}): RelayInfo {
    const r: RelayInfo = {
      id: `relay-${name}`,
      workspace: WORKSPACE,
      zone: this.zoneByName(zone).id,
      name,
      dial_address: `${name}.example.net:7444`,
      host_key_fingerprint: fingerprintOf(RELAY_KEY.replace('GwLab', name)),
      host: null,
      state: 'connected',
      since: NOW,
      rtt_ms: 14,
      version: '0.1.0',
      streams: 0,
      last_error: null,
      revoked_at: null,
      ...over,
    }
    this.relays = [...this.relays, r]
    return r
  }

  /** Puts host `name` in `zone` (`null`: direct) and derives its reach, without pushing. */
  place(name: string, zone: string | null): void {
    const z = zone === null ? null : this.zoneByName(zone).id
    this.hosts.hosts = this.hosts.hosts.map((h) => (h.inventory_name === name ? { ...h, zone: z } : h))
    this.rederive(false)
  }

  /** A relay's link comes up or drops: pushes `RelayChanged` and the reach of every host it moved. */
  setState(name: string, state: RelayState, last_error: string | null = null): void {
    this.relays = this.relays.map((r) =>
      r.name === name
        ? { ...r, state, since: NOW, last_error, rtt_ms: state === 'connected' ? 14 : null, streams: state === 'connected' ? r.streams : 0 }
        : r,
    )
    const r = this.relays.find((x) => x.name === name)
    if (r) this.backend.push({ type: 'RelayChanged', data: r })
    this.rederive(true)
  }

  // --- Model ----------------------------------------------------------------------------------

  private zoneByName(name: string): ZoneInfo {
    const z = this.zones.find((x) => x.name === name)
    if (!z) throw new Error(`no zone ${name}`)
    return z
  }

  private counted(z: ZoneInfo): ZoneInfo {
    return {
      ...z,
      hosts: this.hosts.hosts.filter((h) => h.zone === z.id && h.state !== 'revoked').length,
      relays: this.relays.filter((r) => r.zone === z.id && r.state !== 'revoked').length,
    }
  }

  private zoneList(): ZoneInfo[] {
    return this.zones.map((z) => this.counted(z)).sort((a, b) => a.name.localeCompare(b.name))
  }

  private relayList(): RelayInfo[] {
    return [...this.relays].sort((a, b) => a.name.localeCompare(b.name))
  }

  /**
   * A live host keeps a session while its zone has a working relay (or it is direct). Otherwise
   * the server's `derive_reach` says why it has none.
   */
  private derive(h: HostInfo): { reach: HostReach; online: boolean } {
    if (h.state === 'revoked') return { reach: { kind: 'disconnected' }, online: false }
    if (h.zone === null) {
      return h.state === 'online' ? { reach: { kind: 'connected', path: { kind: 'direct' } }, online: true } : { reach: { kind: 'disconnected' }, online: false }
    }
    const zr = this.relays.filter((r) => r.zone === h.zone)
    const up = zr.find((r) => r.state === 'connected')
    if (up) return { reach: { kind: 'connected', path: { kind: 'relay', relay: up.id } }, online: true }
    const live = zr.filter((r) => r.state !== 'revoked')
    const last = zr.at(-1)
    if (live.length === 0 && last) return { reach: { kind: 'relay_revoked', relay: last.id }, online: false }
    if (live.length === 0) return { reach: { kind: 'no_path' }, online: false }
    if (live.some((r) => r.state === 'connecting')) return { reach: { kind: 'disconnected' }, online: false }
    return { reach: { kind: 'relay_unreachable', relays: live.map((r) => r.id) }, online: false }
  }

  /** Re-derives every host's reach; with `push`, tells the page about the ones that moved. */
  private rederive(push: boolean): void {
    this.hosts.hosts = this.hosts.hosts.map((h) => {
      // A host with a runner installed (`runner_version`) keeps dialling, so it is online whenever
      // its zone gives it a path; one without stays offline.
      const runner = h.runner_version !== null
      const { reach, online } = this.derive({ ...h, state: h.state === 'revoked' ? h.state : runner ? 'online' : 'offline' })
      const state = h.state === 'revoked' ? h.state : online ? 'online' : 'offline'
      const moved = JSON.stringify(reach) !== JSON.stringify(h.reach)
      if (push && state !== h.state) this.backend.push({ type: 'HostState', data: { host: h.id, state } })
      if (push && moved) this.backend.push({ type: 'HostReachChanged', data: { host: h.id, reach } })
      return { ...h, reach, state }
    })
  }

  private preview(action: string, p: Pending, summary: string, affected: string[], warnings: string[] = []): Event[] {
    const preview_id = `relay-preview-${String(++this.seq)}`
    this.pending.set(preview_id, p)
    const data: Preview = { preview_id, action, summary, affected, step_up: STEP_UP.has(p.kind), expires_at: EXPIRES, warnings }
    return [{ type: 'Preview', data }]
  }

  private err(code: string, message: string): Event[] {
    return [{ type: 'OpError', data: { code, message } }]
  }

  private instanceAdmin(): boolean {
    const role = this.backend.me?.instance_role
    return role === 'owner' || role === 'admin'
  }

  private nameTaken(name: string, except: string | null): boolean {
    return this.relays.some((r) => r.name === name && r.id !== except && r.state !== 'revoked')
  }

  private hostNames(ids: (string | null)[]): string[] {
    return this.hosts.hosts.filter((h) => ids.includes(h.id)).map((h) => h.inventory_name)
  }

  /** Hosts whose session comes through `relay` now. */
  private carried(relay: string): HostInfo[] {
    return this.hosts.hosts.filter((h) => h.reach.kind === 'connected' && h.reach.path.kind === 'relay' && h.reach.path.relay === relay)
  }

  private respond(cmd: Cmd): Event[] | undefined {
    switch (cmd.type) {
      case 'ListZones':
        return [{ type: 'Zones', data: { items: this.zoneList().filter((z) => z.workspace === cmd.data.workspace) } }]
      case 'ListRelays':
        return [{ type: 'Relays', data: { items: this.relayList().filter((r) => r.workspace === cmd.data.workspace) } }]
      case 'GetRelayLinkKeys':
        if (!this.instanceAdmin()) return [{ type: 'Denied', data: { action: 'get_relay_link_keys', reason: 'instance admins only' } }]
        return [{ type: 'RelayLinkKeys', data: { items: this.keys } }]

      case 'PreviewZoneCreate': {
        const { name, description } = cmd.data
        if (this.zones.some((z) => z.name === name)) return this.err('zone_exists', `A zone named ${name} already exists.`)
        return this.preview(
          'preview_zone_create',
          { kind: 'zone-create', name, description },
          `Create zone ${name}. Hosts placed in it are reached through its relays; register a relay for it before moving hosts in.`,
          [`zone ${name}`],
        )
      }
      case 'PreviewZoneUpdate': {
        const z = this.zones.find((x) => x.id === cmd.data.zone)
        if (!z) return this.err('not_found', 'No such zone.')
        const { name, description } = cmd.data
        const summary = z.name === name ? `Update zone ${name}'s description.` : `Rename zone ${z.name} to ${name} and update its description.`
        return this.preview('preview_zone_update', { kind: 'zone-update', zone: z.id, name, description }, summary, [`zone ${z.name}`])
      }
      case 'PreviewZoneRemove': {
        const z = this.zones.find((x) => x.id === cmd.data.zone)
        if (!z) return this.err('not_found', 'No such zone.')
        const hosts = this.hosts.hosts.filter((h) => h.zone === z.id && h.state !== 'revoked').map((h) => `host ${h.inventory_name}`)
        const relays = this.relays.filter((r) => r.zone === z.id && r.state !== 'revoked').map((r) => `relay ${r.name}`)
        const blockers = [...hosts, ...relays]
        if (blockers.length === 0) return this.preview('preview_zone_remove', { kind: 'zone-remove', zone: z.id, blocked: false }, `Remove zone ${z.name}.`, [`zone ${z.name}`])
        return this.preview(
          'preview_zone_remove',
          { kind: 'zone-remove', zone: z.id, blocked: true },
          `Zone ${z.name} cannot be removed while hosts or relays point at it. Confirming will be refused.`,
          [`zone ${z.name}`, ...blockers],
          [`In use by ${blockers.join(', ')}.`],
        )
      }
      case 'PreviewHostZone': {
        const h = this.hosts.hosts.find((x) => x.id === cmd.data.host)
        if (!h) return this.err('not_found', 'No such host.')
        const to = cmd.data.zone === null ? null : this.zones.find((z) => z.id === cmd.data.zone)
        if (to === undefined) return this.err('not_found', 'No such zone.')
        const from = h.zone === null ? 'direct' : (this.zones.find((z) => z.id === h.zone)?.name ?? 'direct')
        const toName = to === null ? 'direct' : to.name
        const via = to === null ? 'straight to Keel' : "through one of the new zone's relays"
        const relays = to === null ? 1 : this.relays.filter((r) => r.zone === to.id && r.state !== 'revoked').length
        return this.preview(
          'preview_host_zone',
          { kind: 'host-zone', host: h.id, zone: to?.id ?? null },
          `Move host ${h.inventory_name} from zone ${from} to zone ${toName}. Its runner keeps its session; the next one must come ${via}.`,
          [`host ${h.inventory_name}`],
          relays === 0 ? [`Zone ${toName} has no relay: the host will have no path to Keel.`] : [],
        )
      }

      case 'PreviewRelayRegister':
      case 'PreviewRelayUpdate': {
        const d = cmd.data
        const old = cmd.type === 'PreviewRelayUpdate' ? this.relays.find((r) => r.id === cmd.data.relay) : undefined
        if (cmd.type === 'PreviewRelayUpdate' && !old) return this.err('not_found', 'No such relay.')
        const zone = this.zones.find((z) => z.id === d.zone)
        if (!zone) return this.err('not_found', 'No such zone.')
        if (this.nameTaken(d.name, old?.id ?? null)) return this.err('relay_exists', `A relay named ${d.name} already exists.`)
        const key = d.host_key.trim()
        if (key === '' && !old) return this.err('bad_host_key', 'Paste the relay’s ssh-ed25519 host key.')
        if (key !== '' && !key.startsWith('ssh-ed25519 ')) return this.err('bad_host_key', 'The host key must be one ssh-ed25519 line.')
        const fp = key === '' ? (old?.host_key_fingerprint ?? '') : fingerprintOf(key)
        const relay: RelayInfo = {
          ...(old ?? {
            id: `relay-${d.name}`,
            workspace: zone.workspace,
            state: 'connecting' as const,
            since: NOW,
            rtt_ms: null,
            version: null,
            streams: 0,
            last_error: null,
            revoked_at: null,
          }),
          zone: zone.id,
          name: d.name,
          dial_address: d.dial_address,
          host_key_fingerprint: fp,
          host: d.host,
        }
        if (!old) {
          const active = this.keys.find((k) => k.state === 'active')
          return this.preview(
            'preview_relay_register',
            { kind: 'register', relay },
            `Register relay ${d.name} at ${d.dial_address}, pinning host key ${fp}. Keel dials it at once.`,
            [`relay ${d.name}`],
            [`The relay must trust Keel's relay-link key: add this line to its hub_keys: ${active?.public_line ?? LINK_KEY_LINE}`],
          )
        }
        return this.preview('preview_relay_update', { kind: 'update', relay }, `Update relay ${d.name} (${d.dial_address}, host key ${fp}).`, [`relay ${old.name}`])
      }
      case 'PreviewRelayRevoke': {
        const r = this.relays.find((x) => x.id === cmd.data.relay)
        if (!r) return this.err('not_found', 'No such relay.')
        if (r.state === 'revoked') return this.err('relay_revoked', `Relay ${r.name} is already revoked.`)
        const carried = this.carried(r.id).map((h) => h.inventory_name)
        const sessions =
          carried.length === 0
            ? 'It carries no runner session now.'
            : `The runner sessions it carries end, and their runs in progress are lost: ${carried.join(', ')}.`
        return this.preview(
          'preview_relay_revoke',
          { kind: 'revoke', relay: r.id },
          `Revoke relay ${r.name}: Keel closes its link now and never dials it again. ${sessions} Revocation is final.`,
          [`relay ${r.name}`, ...carried],
        )
      }
      case 'PreviewRelayRelease': {
        const r = this.relays.find((x) => x.id === cmd.data.relay)
        if (!r) return this.err('not_found', 'No such relay.')
        if (r.state !== 'quarantined') return this.err('not_quarantined', `Relay ${r.name} is not quarantined.`)
        return this.preview(
          'preview_relay_release',
          { kind: 'release', relay: r.id },
          `Release relay ${r.name} from quarantine and dial it again. It was quarantined at ${r.since ?? NOW} for ${r.last_error ?? 'a protocol violation'}; 0 refused attempt(s) since.`,
          [`relay ${r.name}`],
          ['A relay is quarantined when it breaks the relay protocol. Release it only once you know why it did.'],
        )
      }

      case 'PreviewRelayLinkKeyRotate':
        if (!this.instanceAdmin()) return [{ type: 'Denied', data: { action: 'rotate_relay_link_key', reason: 'instance admins only' } }]
        if (this.keys.some((k) => k.state === 'pending')) return this.err('key_pending', 'A pending key exists: activate or retire it first.')
        return this.preview(
          'preview_relay_link_key_rotate',
          { kind: 'rotate' },
          "Make a new relay-link key, pending. Keel keeps dialling with the active key until you activate the new one; add its public line to every relay's hub_keys first.",
          ['relay-link key'],
        )
      case 'PreviewRelayLinkKeyActivate': {
        const k = this.keys.find((x) => x.fingerprint === cmd.data.fingerprint && x.state === 'pending')
        if (!k) return this.err('not_found', 'No such pending key.')
        const names = this.relays.filter((r) => r.state !== 'revoked').map((r) => r.name)
        return this.preview(
          'preview_relay_link_key_activate',
          { kind: 'activate', fingerprint: k.fingerprint },
          `Activate relay-link key ${k.fingerprint} and retire the active one. Links up now stay up; every new dial uses the new key.`,
          ['relay-link key'],
          ["Every relay's hub_keys must list the new key's line first, or Keel's next dial to it is refused.", ...(names.length ? [`Relays: ${names.join(', ')}.`] : [])],
        )
      }
      case 'PreviewRelayLinkKeyRetire': {
        const k = this.keys.find((x) => x.fingerprint === cmd.data.fingerprint && x.state === 'pending')
        if (!k) return this.err('not_found', 'No such pending key.')
        return this.preview('preview_relay_link_key_retire', { kind: 'retire', fingerprint: k.fingerprint }, `Retire the pending relay-link key ${k.fingerprint}. The active key stays.`, ['relay-link key'])
      }

      case 'PreviewRunStart': {
        const run = this.hosts.hosts.filter((h) => h.state !== 'revoked')
        const warnings: string[] = []
        for (const r of this.relays) {
          if (r.state === 'revoked' || r.host === null || !run.some((h) => h.id === r.host)) continue
          if (this.relays.some((o) => o.id !== r.id && o.zone === r.zone && o.state !== 'revoked')) continue
          const behind = run.filter((h) => h.zone === r.zone && h.id !== r.host).map((h) => h.inventory_name)
          if (behind.length === 0) continue
          const relayHost = this.hostNames([r.host])[0] ?? 'a host'
          warnings.push(
            `This run includes ${relayHost}, which runs relay ${r.name}, the only relay for ${behind.join(', ')}. If the run restarts or breaks the relay, those hosts lose their path to Keel and their part of the run is lost. Consider running ${relayHost} separately.`,
          )
        }
        const job = cmd.data.job.replace(/^job-/, '')
        return this.preview('preview_run_start', { kind: 'run' }, `Run ${job} on ${String(run.length)} host(s).`, run.map((h) => h.inventory_name), warnings)
      }

      case 'Confirm':
        return this.confirm(cmd.data.preview_id, cmd.data.step_up !== undefined)
      default:
        return undefined
    }
  }

  private confirm(preview_id: string, assertion: boolean): Event[] | undefined {
    const p = this.pending.get(preview_id)
    if (p === undefined) return undefined
    if (STEP_UP.has(p.kind) && !assertion && !this.backend.me?.verification.verified) {
      return this.err('step_up_required', 'Confirm it’s you with a passkey first.')
    }
    this.pending.delete(preview_id)
    const ok: Event = { type: 'Confirmed', data: { preview_id } }
    const zones = (): Event => ({ type: 'Zones', data: { items: this.zoneList() } })
    const relays = (): Event => ({ type: 'Relays', data: { items: this.relayList() } })
    const keys = (): Event => ({ type: 'RelayLinkKeys', data: { items: this.keys } })
    switch (p.kind) {
      case 'zone-create':
        this.zone(p.name, p.description)
        return [ok, zones()]
      case 'zone-update':
        this.zones = this.zones.map((z) => (z.id === p.zone ? { ...z, name: p.name, description: p.description } : z))
        return [ok, zones()]
      case 'zone-remove':
        if (p.blocked) return this.err('zone_in_use', 'The zone is in use by hosts or relays.')
        this.zones = this.zones.filter((z) => z.id !== p.zone)
        return [ok, zones()]
      case 'host-zone': {
        this.hosts.hosts = this.hosts.hosts.map((h) => (h.id === p.host ? { ...h, zone: p.zone } : h))
        this.rederive(true)
        const host = this.hosts.hosts.find((h) => h.id === p.host)
        if (!host) return this.err('not_found', 'No such host.')
        return [
          ok,
          { type: 'Host', data: { host, python_version: null, override_quorum: null, ledger_head: null, certs: [], outside_runs: [] } },
        ]
      }
      case 'register':
      case 'update': {
        const exists = this.relays.some((r) => r.id === p.relay.id)
        this.relays = exists ? this.relays.map((r) => (r.id === p.relay.id ? p.relay : r)) : [...this.relays, p.relay]
        this.backend.push({ type: 'RelayChanged', data: p.relay })
        return [ok, relays()]
      }
      case 'revoke':
      case 'release': {
        const state: RelayState = p.kind === 'revoke' ? 'revoked' : 'connecting'
        this.relays = this.relays.map((r) =>
          r.id === p.relay ? { ...r, state, since: NOW, revoked_at: p.kind === 'revoke' ? NOW : r.revoked_at, rtt_ms: null, streams: 0, last_error: p.kind === 'release' ? null : r.last_error } : r,
        )
        const r = this.relays.find((x) => x.id === p.relay)
        if (r) this.backend.push({ type: 'RelayChanged', data: r })
        this.rederive(true)
        return [ok, relays()]
      }
      case 'rotate':
        this.keys = [
          ...this.keys,
          {
            fingerprint: 'SHA256:keelrelaylinkkeynext000000000000000000000000',
            public_line: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKeelRelayLinkNext keel-relay-link',
            state: 'pending',
          },
        ]
        return [ok, keys()]
      case 'activate':
        this.keys = this.keys.map((k) =>
          k.fingerprint === p.fingerprint ? { ...k, state: 'active' } : k.state === 'active' ? { ...k, state: 'retired' } : k,
        )
        return [ok, keys()]
      case 'retire':
        this.keys = this.keys.map((k) => (k.fingerprint === p.fingerprint ? { ...k, state: 'retired' } : k))
        return [ok, keys()]
      case 'run':
        return [ok]
    }
  }
}
