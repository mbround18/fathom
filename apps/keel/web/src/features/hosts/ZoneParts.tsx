/**
 * The Hosts page's zone pieces (spec 067 US3, T034; contracts/ui.md "Hosts"): the Path cell, the
 * relay chips in a zone group's header, and the inspector's zone with Change zone.
 */
import {
  ArrowsLeftRightIcon,
  CircleNotchIcon,
  PlugsConnectedIcon,
  ProhibitIcon,
  WarningIcon,
  XCircleIcon,
} from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { LABEL, Select } from '@companion/ui/kit'
import { useId, useState, type ReactNode } from 'react'

import { useSession } from '@/app/session-store'
import { useRelays } from '@/features/settings/relays-store'
import type { HostInfo, RelayInfo, RelayState } from '@/protocol/wire'

import { pathText, reachDetail, reachText, reachTone, relayTone, rtt, streams, TONE, zoneName, type ReachTone, type ZoneGroup } from './reach'
import { canAdminister } from './workspaces'

const BORDER: Record<ReachTone, string> = {
  ok: 'border-status-ok/40',
  quiet: 'border-border',
  warn: 'border-status-warn/60',
  err: 'border-status-err/60',
}

const RELAY_ICON: Record<RelayState, ReactNode> = {
  connected: <PlugsConnectedIcon aria-hidden weight="bold" />,
  connecting: <CircleNotchIcon aria-hidden weight="bold" className="motion-safe:animate-spin" />,
  unreachable: <WarningIcon aria-hidden weight="bold" />,
  quarantined: <ProhibitIcon aria-hidden weight="bold" />,
  revoked: <XCircleIcon aria-hidden weight="bold" />,
}

/** The Path column: `direct`, `via gw-lab`, or why the host has no path. */
export function PathCell({ host, relays }: { host: HostInfo; relays: readonly RelayInfo[] }) {
  const tone = reachTone(host.reach)
  return (
    <span
      data-host-path={host.reach.kind}
      title={reachDetail(host.reach, relays) ?? reachText(host.reach, relays)}
      className={`text-xs break-normal [overflow-wrap:normal] ${tone === 'ok' ? 'font-mono text-foreground' : `font-medium ${TONE[tone]}`}`}
    >
      {pathText(host.reach, relays)}
    </span>
  )
}

/**
 * One relay as a chip: an icon and colour for its state, its name always, and RTT and streams from
 * 768 px up, so a phone keeps one line per chip.
 */
export function RelayChip({ relay }: { relay: RelayInfo }) {
  const tone = relayTone(relay.state)
  const extra = [rtt(relay), streams(relay)].filter((x) => x !== null)
  const label = [`Relay ${relay.name}`, relay.state, ...extra].join(', ')
  return (
    <li
      aria-label={label}
      title={relay.last_error ?? label}
      data-relay={relay.name}
      data-relay-state={relay.state}
      className={`inline-flex min-h-6 max-w-full min-w-0 items-center gap-1 border px-1.5 text-xs ${BORDER[tone]}`}
    >
      <span className={`inline-flex shrink-0 ${TONE[tone]}`}>{RELAY_ICON[relay.state]}</span>
      <span className="truncate font-mono">{relay.name}</span>
      <span className={`font-medium ${TONE[tone]}`}>{relay.state}</span>
      {extra.length > 0 && <span className="hidden whitespace-nowrap text-muted-foreground tabular-nums md:inline">· {extra.join(' · ')}</span>}
    </li>
  )
}

/** A zone group's header: its name, how many hosts, and its relays (none: no path). */
export function ZoneHeader({ group, headingId }: { group: ZoneGroup; headingId: string }) {
  const live = group.relays.filter((r) => r.state !== 'revoked')
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5">
      <h3 id={headingId} className="flex min-w-0 items-baseline gap-2 text-sm font-semibold">
        <span className="font-mono">{group.name}</span>
        <span className="text-xs font-normal text-muted-foreground tabular-nums">
          {group.hosts.length === 1 ? '1 host' : `${String(group.hosts.length)} hosts`}
        </span>
      </h3>
      {group.id !== null &&
        (group.relays.length === 0 ? (
          <span data-zone-no-relay className="text-xs font-medium text-status-err">
            No relay: hosts here have no path to Keel.
          </span>
        ) : (
          <ul aria-label={`Relays for ${group.name}`} className="flex min-w-0 flex-wrap gap-1.5">
            {group.relays.map((r) => (
              <RelayChip key={r.id} relay={r} />
            ))}
          </ul>
        ))}
      {group.id !== null && group.relays.length > 0 && live.length === 0 && (
        <span className="text-xs font-medium text-status-err">Every relay is revoked.</span>
      )}
    </div>
  )
}

/**
 * The inspector's zone and path, and Change zone for a workspace admin: pick a zone, then the
 * usual preview (which warns when the zone has no relay) and confirm.
 */
export function HostZone({ host }: { host: HostInfo }) {
  const me = useSession((s) => s.me)
  const zones = useRelays((s) => s.zones.value) ?? []
  const relays = useRelays((s) => s.relays.value) ?? []
  const setHostZone = useRelays((s) => s.setHostZone)
  const [picked, setPicked] = useState<{ host: string; zone: string } | null>(null)
  const id = useId()
  const current = host.zone ?? ''
  const value = picked?.host === host.id ? picked.zone : current
  const canChange = canAdminister(me, host.workspace) && host.state !== 'revoked' && (zones.length > 0 || host.zone !== null)
  return (
    <div data-host-zone className="flex min-w-0 flex-col gap-1">
      <div className="grid min-w-0 grid-cols-[auto_1fr] gap-x-3 gap-y-1">
        <span className={LABEL}>Zone</span>
        <span className="font-mono">{zoneName(host.zone, zones)}</span>
        <span className={LABEL}>Path</span>
        <span className={TONE[reachTone(host.reach)]}>{reachText(host.reach, relays)}</span>
      </div>
      {reachDetail(host.reach, relays) && <p className="text-muted-foreground">{reachDetail(host.reach, relays)}</p>}
      {canChange && (
        <div className="flex min-w-0 flex-wrap items-center gap-2 pt-1">
          <label htmlFor={id} className="sr-only">
            New zone for {host.inventory_name}
          </label>
          <Select
            id={id}
            dense
            className="min-w-0 flex-1 sm:max-w-56 sm:flex-none"
            value={value}
            onChange={(e) => {
              setPicked({ host: host.id, zone: e.target.value })
            }}
          >
            <option value="">direct</option>
            {zones.map((z) => (
              <option key={z.id} value={z.id}>
                {z.name}
              </option>
            ))}
          </Select>
          <Button
            size="sm"
            variant="outline"
            disabled={value === current}
            onClick={() => {
              setHostZone(host, value === '' ? null : value)
            }}
          >
            <ArrowsLeftRightIcon aria-hidden />
            Change zone
          </Button>
        </div>
      )}
    </div>
  )
}
