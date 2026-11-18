/**
 * Zones, relays and Keel's relay-link key (spec 067 US3, T035; contracts/ui.md "Settings →
 * Relays"). Settings → Relays manages them; the Hosts page reads the same zones and relays to
 * group hosts and name the relay each one comes through.
 *
 * Reads: `ListZones` and `ListRelays` (View on the workspace) and `GetRelayLinkKeys` (instance
 * admins). Pushes keep them current: `Zones` after a zone change, `RelayChanged` on a relay's state,
 * RTT or stream count. `HostReachChanged` belongs to the hosts store.
 *
 * Every change goes through the governance preview → confirm dialog (`useGovernance.propose`),
 * which asks for a passkey when the preview says so (register, edit, release and the link-key
 * steps). Revoke asks for none, so a compromised relay is cut off in two clicks. A confirm answers
 * the new list (`Zones`, `Relays`, `RelayLinkKeys`, or `Host` for a zone move), which lands here.
 *
 * The register / edit form keeps its draft here, so it survives stepping aside for the preview and
 * comes back as it was when the server refuses.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent } from '@/app/connection'
import { loadList } from '@/app/load'
import { useGovernance } from '@/features/teams/store'
import type { Cmd, Event, RelayInfo, RelayLinkKey, ZoneInfo } from '@/protocol/wire'

import type { Loaded } from './store'

const EMPTY = { value: null, refusal: null }

/** A zone or relay name (keel-protocol `ZoneName`): lowercase, digits and dashes, not `direct`. */
export const ZONE_NAME = /^[a-z0-9][a-z0-9-]{0,62}$/

export function validName(name: string): boolean {
  return ZONE_NAME.test(name) && name !== 'direct'
}

/** `host:port` (or `[v6]:port`), port 1–65535, no scheme or path. The server parses it again. */
export function validDialAddress(addr: string): boolean {
  const m = /^(\[[0-9a-fA-F:.]+\]|[A-Za-z0-9.-]+):(\d{1,5})$/.exec(addr)
  if (!m) return false
  const port = Number(m[2])
  return port >= 1 && port <= 65535
}

/** One `ssh-ed25519 AAAA… [comment]` line; any other algorithm is refused. */
export function validHostKey(line: string): boolean {
  return /^ssh-ed25519 AAAA[A-Za-z0-9+/=]+( [^\n]*)?$/.test(line.trim())
}

export interface RelayDraft {
  name: string
  zone: string
  dial_address: string
  host_key: string
  /** The enrolled host the relay runs on, if any. */
  host: string | null
}

export interface RelayEditor {
  /** `null` registers a new relay. */
  relay: RelayInfo | null
  draft: RelayDraft
}

export interface RelaysState {
  workspace: string | null
  zones: Loaded<ZoneInfo[]>
  relays: Loaded<RelayInfo[]>
  keys: Loaded<RelayLinkKey[]>
  editor: RelayEditor | null

  load: (workspace: string) => void
  loadKeys: () => void

  createZone: (name: string, description: string, onDone?: () => void) => void
  updateZone: (zone: ZoneInfo, name: string, description: string, onDone?: () => void) => void
  removeZone: (zone: ZoneInfo) => void
  /** Moves `host` into `zone` (`null`: direct). */
  setHostZone: (host: { id: string; inventory_name: string }, zone: string | null) => void

  openEditor: (relay: RelayInfo | null) => void
  editDraft: (change: Partial<RelayDraft>) => void
  closeEditor: () => void
  /** Previews the open editor's register or update. */
  saveRelay: () => void
  revokeRelay: (relay: RelayInfo) => void
  releaseRelay: (relay: RelayInfo) => void

  rotateKey: () => void
  activateKey: (key: RelayLinkKey) => void
  retireKey: (key: RelayLinkKey) => void
}

/** What has been asked for, so a reconnect asks again. */
let keysAsked = false

function readZones(workspace: string): void {
  const was = useRelays.getState().zones
  if (was.refusal) useRelays.setState({ zones: { value: was.value, refusal: null } })
  loadList(
    { type: 'ListZones', data: { workspace } },
    (e) => (e.type === 'Zones' ? e.data.items : undefined),
    {
      loaded: (value) => {
        if (useRelays.getState().workspace === workspace) useRelays.setState({ zones: { value, refusal: null } })
      },
      failed: (refusal) => {
        if (useRelays.getState().workspace === workspace) useRelays.setState({ zones: { value: useRelays.getState().zones.value, refusal } })
      },
    },
  )
}

function readRelays(workspace: string): void {
  const was = useRelays.getState().relays
  if (was.refusal) useRelays.setState({ relays: { value: was.value, refusal: null } })
  loadList(
    { type: 'ListRelays', data: { workspace } },
    (e) => (e.type === 'Relays' ? e.data.items : undefined),
    {
      loaded: (value) => {
        if (useRelays.getState().workspace === workspace) useRelays.setState({ relays: { value, refusal: null } })
      },
      failed: (refusal) => {
        if (useRelays.getState().workspace === workspace) useRelays.setState({ relays: { value: useRelays.getState().relays.value, refusal } })
      },
    },
  )
}

/** Applies what a confirm answered: the new zones, relays or keys. */
function applied(event: Event): void {
  const ws = useRelays.getState().workspace
  switch (event.type) {
    case 'Zones':
      useRelays.setState({ zones: { value: event.data.items.filter((z) => z.workspace === ws), refusal: null } })
      break
    case 'Relays':
      useRelays.setState({ relays: { value: event.data.items.filter((r) => r.workspace === ws), refusal: null } })
      break
    case 'RelayLinkKeys':
      useRelays.setState({ keys: { value: event.data.items, refusal: null } })
      break
    default:
      break
  }
}

function propose(title: string, cmd: Cmd, after?: (event: Event) => void): void {
  useGovernance.getState().propose(title, cmd, (event) => {
    applied(event)
    after?.(event)
  })
}

function upsert(items: RelayInfo[], relay: RelayInfo): RelayInfo[] {
  if (items.some((r) => r.id === relay.id)) return items.map((r) => (r.id === relay.id ? relay : r))
  return [...items, relay].sort((a, b) => a.name.localeCompare(b.name))
}

export const useRelays = create<RelaysState>()((set, get) => ({
  workspace: null,
  zones: EMPTY,
  relays: EMPTY,
  keys: EMPTY,
  editor: null,

  load: (workspace) => {
    if (get().workspace !== workspace) set({ workspace, zones: EMPTY, relays: EMPTY, editor: null })
    readZones(workspace)
    readRelays(workspace)
  },

  loadKeys: () => {
    keysAsked = true
    const was = get().keys
    if (was.refusal) set({ keys: { value: was.value, refusal: null } })
    loadList({ type: 'GetRelayLinkKeys' }, (e) => (e.type === 'RelayLinkKeys' ? e.data.items : undefined), {
      loaded: (value) => {
        set({ keys: { value, refusal: null } })
      },
      failed: (refusal) => {
        set({ keys: { value: get().keys.value, refusal } })
      },
    })
  },

  createZone: (name, description, onDone) => {
    const workspace = get().workspace
    if (workspace === null) return
    propose(`Create zone ${name}`, { type: 'PreviewZoneCreate', data: { workspace, name, description } }, (e) => {
      if (e.type === 'Zones') onDone?.()
    })
  },

  updateZone: (zone, name, description, onDone) => {
    propose(zone.name === name ? `Edit zone ${name}` : `Rename zone ${zone.name}`, { type: 'PreviewZoneUpdate', data: { zone: zone.id, name, description } }, (e) => {
      if (e.type === 'Zones') onDone?.()
    })
  },

  removeZone: (zone) => {
    propose(`Remove zone ${zone.name}`, { type: 'PreviewZoneRemove', data: { zone: zone.id } })
  },

  setHostZone: (host, zone) => {
    // The confirm answers `Host`, which the hosts store applies; zone counts change with it.
    propose(`Change zone of ${host.inventory_name}`, { type: 'PreviewHostZone', data: { host: host.id, zone } }, (e) => {
      const ws = get().workspace
      if (e.type === 'Confirmed' && ws !== null) readZones(ws)
    })
  },

  openEditor: (relay) => {
    const zones = get().zones.value ?? []
    set({
      editor: {
        relay,
        draft: relay
          ? { name: relay.name, zone: relay.zone, dial_address: relay.dial_address, host_key: '', host: relay.host }
          : { name: '', zone: zones[0]?.id ?? '', dial_address: '', host_key: '', host: null },
      },
    })
  },

  editDraft: (change) => {
    const e = get().editor
    if (e) set({ editor: { ...e, draft: { ...e.draft, ...change } } })
  },

  closeEditor: () => {
    set({ editor: null })
  },

  saveRelay: () => {
    const e = get().editor
    const workspace = get().workspace
    if (!e || workspace === null) return
    const { name, zone, dial_address, host } = e.draft
    const host_key = e.draft.host_key.trim()
    const done = (event: Event) => {
      if (event.type === 'Relays') set({ editor: null })
    }
    if (e.relay) {
      propose(`Edit relay ${e.relay.name}`, { type: 'PreviewRelayUpdate', data: { relay: e.relay.id, zone, name, dial_address, host_key, host } }, done)
    } else {
      propose(`Register relay ${name}`, { type: 'PreviewRelayRegister', data: { workspace, zone, name, dial_address, host_key, host } }, done)
    }
  },

  revokeRelay: (relay) => {
    propose(`Revoke relay ${relay.name}`, { type: 'PreviewRelayRevoke', data: { relay: relay.id } })
  },

  releaseRelay: (relay) => {
    propose(`Release relay ${relay.name}`, { type: 'PreviewRelayRelease', data: { relay: relay.id } })
  },

  rotateKey: () => {
    propose('Rotate relay-link key', { type: 'PreviewRelayLinkKeyRotate' })
  },

  activateKey: (key) => {
    propose('Activate relay-link key', { type: 'PreviewRelayLinkKeyActivate', data: { fingerprint: key.fingerprint } })
  },

  retireKey: (key) => {
    propose('Retire relay-link key', { type: 'PreviewRelayLinkKeyRetire', data: { fingerprint: key.fingerprint } })
  },
}))

onServerEvent(({ reply_to, event }) => {
  const s = useRelays.getState()
  if (s.workspace === null) return
  switch (event.type) {
    case 'RelayChanged':
      if (event.data.workspace === s.workspace) {
        useRelays.setState({ relays: { value: upsert(s.relays.value ?? [], event.data), refusal: s.relays.refusal } })
      }
      break
    case 'Zones': {
      // Replies land through `loadList` or a confirm; only a push is handled here.
      if (reply_to !== undefined) break
      const first = event.data.items[0]
      if (first === undefined) readZones(s.workspace)
      else if (first.workspace === s.workspace) useRelays.setState({ zones: { value: event.data.items, refusal: null } })
      break
    }
    default:
      break
  }
})

onReconnected(() => {
  const { workspace, load, loadKeys } = useRelays.getState()
  if (workspace) load(workspace)
  if (keysAsked) loadKeys()
})
