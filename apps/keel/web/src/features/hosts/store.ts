/**
 * Hosts (spec 055 US7, T088): the workspace's enrolled hosts and the SSH CA line runners pin.
 *
 * Lists follow what the server sends: `Hosts`, `Host` and the pushed `HostState` (online, offline,
 * revoked). Revoking is two-step, like every change: `PreviewHostRevoke` answers a `Preview`,
 * shown in a confirm dialog, then `Confirm` (with a passkey assertion when the preview asks for
 * step-up and the session has not just verified). `Preview`, `OpError` and `Denied` count only when
 * they answer a command sent from here.
 *
 * Enrolling lives in the Enroll dialog (spec 060, `enroll-store.ts`), which holds the minted token.
 *
 * Changed outside Keel (spec 058 amendment A2): `HostInfo.changed_outside` marks a host whose
 * runner reported a run Keel did not make. The list can be narrowed to those hosts (the server
 * filters), and one host's detail (`GetHost`) carries the newest entries it reported. The pushed
 * `HostChangedOutside` says only that the flag moved, so the list and an open detail are re-read.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { loadList, type Refusal } from '@/app/load'
import { useSession } from '@/app/session-store'
import { usePasskeys } from '@/features/passkeys/store'
import type { Cmd, HostDetail, HostInfo, Preview, RequestId, SshCaInfo } from '@/protocol/wire'

const OFFLINE = 'Keel is not connected. Wait for it to reconnect, then try again.'

export interface SshCa {
  cas: SshCaInfo[]
  /** The `@cert-authority * …` line for `keel_runner_host_ca`. */
  cert_authority: string
}

/** Splits "debian, web" or "debian web" into profile names. */
export function parseProfiles(text: string): string[] {
  return [...new Set(text.split(/[\s,]+/).filter((p) => p.length > 0))]
}

export interface HostsState {
  workspace: string | null
  hosts: HostInfo[]
  /** Whether `hosts` has answered for this workspace, so an empty list means no hosts. */
  loaded: boolean
  /** Why the list failed to load, until it loads. */
  failed: Refusal | null
  ssh: SshCa | null
  preview: Preview | null
  error: string | null
  /** List only hosts changed outside Keel. */
  changedOnly: boolean
  /** The host whose detail is open, and its detail once it arrives. */
  open: string | null
  detail: HostDetail | null
  load: (workspace: string) => void
  setChangedOnly: (on: boolean) => void
  /** Opens `host`'s detail (`GetHost`); `null` closes it. */
  show: (host: string | null) => void
  revoke: (host: string) => void
  confirm: () => Promise<void>
  cancel: () => void
  clearError: () => void
}

const mine = new Set<RequestId>()

function send(cmd: Cmd): boolean {
  const id = sendCmd(cmd)
  if (id === null) {
    useHosts.setState({ error: OFFLINE })
    return false
  }
  mine.add(id)
  return true
}

function list(workspace: string): void {
  const changed_outside = useHosts.getState().changedOnly ? true : null
  useHosts.setState({ failed: null })
  loadList(
    { type: 'ListHosts', data: { workspace, profile: null, drift: null, changed_outside } },
    (event) => (event.type === 'Hosts' ? event.data.items : undefined),
    {
      loaded: (hosts) => {
        if (useHosts.getState().workspace === workspace) useHosts.setState({ hosts, loaded: true, failed: null })
      },
      failed: (failed) => {
        if (useHosts.getState().workspace === workspace) useHosts.setState({ failed })
      },
    },
  )
}

export const useHosts = create<HostsState>()((set, get) => ({
  workspace: null,
  hosts: [],
  loaded: false,
  failed: null,
  ssh: null,
  preview: null,
  error: null,
  changedOnly: false,
  open: null,
  detail: null,

  load: (workspace) => {
    if (get().workspace !== workspace) set({ workspace, hosts: [], loaded: false, open: null, detail: null })
    list(workspace)
    send({ type: 'GetSshCa' })
  },
  setChangedOnly: (on) => {
    set({ changedOnly: on })
    const workspace = get().workspace
    if (workspace) list(workspace)
  },
  show: (host) => {
    set({ open: host, detail: null })
    if (host !== null) send({ type: 'GetHost', data: { host } })
  },
  revoke: (host) => {
    set({ error: null, preview: null })
    send({ type: 'PreviewHostRevoke', data: { host } })
  },
  confirm: async () => {
    const p = get().preview
    if (!p) return
    const verified = useSession.getState().me?.verification.verified ?? false
    if (p.step_up && !verified) {
      const assertion = await usePasskeys.getState().requestStepUp(p.summary)
      if (!assertion) return
      send({ type: 'Confirm', data: { preview_id: p.preview_id, step_up: assertion } })
    } else {
      send({ type: 'Confirm', data: { preview_id: p.preview_id } })
    }
  },
  cancel: () => {
    set({ preview: null })
  },
  clearError: () => {
    set({ error: null })
  },
}))

function upsert(items: HostInfo[], host: HostInfo): HostInfo[] {
  if (items.some((x) => x.id === host.id)) return items.map((x) => (x.id === host.id ? host : x))
  return [...items, host].sort((a, b) => a.inventory_name.localeCompare(b.inventory_name))
}

onServerEvent(({ reply_to, event }) => {
  const s = useHosts.getState()
  const ours = reply_to !== undefined && mine.has(reply_to)
  switch (event.type) {
    case 'Host': {
      const { host } = event.data
      if (host.workspace !== s.workspace) break
      // Under the filter, a host no longer changed outside Keel leaves the list.
      const hosts =
        s.changedOnly && !host.changed_outside ? s.hosts.filter((h) => h.id !== host.id) : upsert(s.hosts, host)
      useHosts.setState(s.open === host.id ? { hosts, detail: event.data } : { hosts })
      break
    }
    case 'HostChangedOutside': {
      const { host } = event.data
      if (s.workspace) list(s.workspace)
      if (s.open === host) send({ type: 'GetHost', data: { host } })
      break
    }
    case 'HostState': {
      const { host, state } = event.data
      if (s.hosts.some((h) => h.id === host)) {
        useHosts.setState({ hosts: s.hosts.map((h) => (h.id === host ? { ...h, state } : h)) })
      } else if (s.workspace) {
        // A host this list has not seen yet (just enrolled): re-list.
        list(s.workspace)
      }
      break
    }
    case 'SshCa':
      if (ours) useHosts.setState({ ssh: { cas: event.data.cas, cert_authority: event.data.cert_authority } })
      break
    case 'Preview':
      if (ours) useHosts.setState({ preview: event.data, error: null })
      break
    case 'Confirmed':
      if (ours) useHosts.setState({ preview: null })
      break
    case 'OpError':
      if (ours) useHosts.setState({ error: event.data.message, preview: null })
      break
    case 'Denied':
      if (ours) useHosts.setState({ error: `Not allowed: ${event.data.reason}`, preview: null })
      break
    default:
      break
  }
})

onReconnected(() => {
  const { workspace, load } = useHosts.getState()
  if (workspace) load(workspace)
})
