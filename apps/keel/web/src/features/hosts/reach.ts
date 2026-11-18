/**
 * Zones, relays and how each host reaches Keel, for display (spec 067 US3, data-model.md "Host
 * (display, derived)"). The server derives `HostInfo.reach`; this only words it and groups hosts
 * by zone. Relays are named by id on the wire, so every helper takes the workspace's relays to
 * name them, and falls back to "a relay" for one the list has not seen yet.
 */
import type { HostInfo, HostReach, RelayInfo, RelayState, ZoneInfo } from '@/protocol/wire'

/** The implicit zone of hosts with no zone: they dial Keel straight. */
export const DIRECT = 'direct'

/** How a reach reads: `ok` connected, `quiet` disconnected, `warn` its relay is down, `err` no way in. */
export type ReachTone = 'ok' | 'quiet' | 'warn' | 'err'

/** Each tone's text colour. */
export const TONE: Record<ReachTone, string> = {
  ok: 'text-status-ok',
  quiet: 'text-muted-foreground',
  warn: 'text-status-warn',
  err: 'text-status-err',
}

function relayName(id: string, relays: readonly RelayInfo[]): string {
  return relays.find((r) => r.id === id)?.name ?? 'a relay'
}

/**
 * The Path column: `direct` or `via <relay>` while connected, else why not
 * (`disconnected (relay unreachable)`, `no path`, …).
 */
export function pathText(reach: HostReach, relays: readonly RelayInfo[]): string {
  switch (reach.kind) {
    case 'connected':
      return reach.path.kind === 'direct' ? 'direct' : `via ${relayName(reach.path.relay, relays)}`
    case 'disconnected':
      return 'disconnected'
    case 'relay_unreachable':
      return 'disconnected (relay unreachable)'
    case 'relay_revoked':
      return 'disconnected (relay revoked)'
    case 'no_path':
      return 'no path'
  }
}

/** The full reach, as data-model.md words it: `connected · via r1`, `no path`, … */
export function reachText(reach: HostReach, relays: readonly RelayInfo[]): string {
  return reach.kind === 'connected' ? `connected · ${pathText(reach, relays)}` : pathText(reach, relays)
}

/** One line on why a host cannot be reached, when there is more to say than the Path column. */
export function reachDetail(reach: HostReach, relays: readonly RelayInfo[]): string | null {
  switch (reach.kind) {
    case 'relay_unreachable':
      return `Keel cannot reach ${reach.relays.map((r) => relayName(r, relays)).join(', ')}, the zone's relays.`
    case 'relay_revoked':
      return `Its last session came through ${relayName(reach.relay, relays)}, which is revoked.`
    case 'no_path':
      return 'Its zone has no relay, so the runner has no way to reach Keel.'
    default:
      return null
  }
}

export function reachTone(reach: HostReach): ReachTone {
  switch (reach.kind) {
    case 'connected':
      return 'ok'
    case 'disconnected':
      return 'quiet'
    case 'relay_unreachable':
      return 'warn'
    case 'relay_revoked':
    case 'no_path':
      return 'err'
  }
}

/** Relay states as they read, worst last so a sort puts working relays first. */
const STATE_ORDER: Record<RelayState, number> = { connected: 0, connecting: 1, unreachable: 2, quarantined: 3, revoked: 4 }

export function relayTone(state: RelayState): ReachTone {
  switch (state) {
    case 'connected':
      return 'ok'
    case 'connecting':
      return 'quiet'
    case 'unreachable':
      return 'warn'
    case 'quarantined':
    case 'revoked':
      return 'err'
  }
}

export interface ZoneGroup {
  /** `null` for `direct`. */
  id: string | null
  name: string
  description: string
  hosts: HostInfo[]
  /** The zone's relays, working ones first; none for `direct`. */
  relays: RelayInfo[]
}

/** Whether the Hosts page groups at all: once the workspace has a zone, or a host sits in one. */
export function hasZones(zones: readonly ZoneInfo[], hosts: readonly HostInfo[]): boolean {
  return zones.length > 0 || hosts.some((h) => h.zone !== null)
}

/**
 * Hosts by zone: `direct` first, then zones by name. A zone shows even with no hosts listed,
 * unless the list is `filtered` (then only zones with a matching host do). A host in a zone the
 * list has not seen yet gets a group of its own, named "unknown zone".
 */
export function groupHosts(
  hosts: readonly HostInfo[],
  zones: readonly ZoneInfo[],
  relays: readonly RelayInfo[],
  filtered = false,
): ZoneGroup[] {
  const byZone = new Map<string | null, HostInfo[]>()
  for (const h of hosts) byZone.set(h.zone, [...(byZone.get(h.zone) ?? []), h])
  const groups: ZoneGroup[] = []
  const direct = byZone.get(null) ?? []
  if (direct.length > 0 || (!filtered && zones.length === 0)) {
    groups.push({ id: null, name: DIRECT, description: 'Hosts whose runner dials Keel straight.', hosts: direct, relays: [] })
  }
  const known = new Set(zones.map((z) => z.id))
  const named = [...zones].sort((a, b) => a.name.localeCompare(b.name))
  for (const z of named) {
    const mine = byZone.get(z.id) ?? []
    if (filtered && mine.length === 0) continue
    const zr = relays
      .filter((r) => r.zone === z.id)
      .sort((a, b) => STATE_ORDER[a.state] - STATE_ORDER[b.state] || a.name.localeCompare(b.name))
    groups.push({ id: z.id, name: z.name, description: z.description, hosts: mine, relays: zr })
  }
  for (const [id, mine] of byZone) {
    if (id === null || known.has(id)) continue
    groups.push({ id, name: 'unknown zone', description: '', hosts: mine, relays: relays.filter((r) => r.zone === id) })
  }
  return groups
}

/** A relay's round-trip time, or `null` when Keel has not measured one. */
export function rtt(relay: RelayInfo): string | null {
  return relay.rtt_ms === null ? null : `${String(relay.rtt_ms)} ms`
}

/** "1 stream", "3 streams", or `null` when unknown. */
export function streams(relay: RelayInfo): string | null {
  if (relay.streams === null) return null
  return relay.streams === 1 ? '1 stream' : `${String(relay.streams)} streams`
}

/** The zone's name for `zone` (`null`: direct). */
export function zoneName(zone: string | null, zones: readonly ZoneInfo[]): string {
  if (zone === null) return DIRECT
  return zones.find((z) => z.id === zone)?.name ?? 'unknown zone'
}
