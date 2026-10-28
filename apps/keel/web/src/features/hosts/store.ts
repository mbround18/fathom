/**
 * Hosts (spec 055 US7, T088): the workspace's enrolled hosts, the SSH CA line runners pin, and
 * the one enrolment token just minted.
 *
 * Lists follow what the server sends: `Hosts`, `Host` and the pushed `HostState` (online, offline,
 * revoked). Enrolling and revoking are two-step, like every change: `PreviewHostEnroll` /
 * `PreviewHostRevoke` answer a `Preview`, shown in a confirm dialog, then `Confirm` (with a passkey
 * assertion when the preview asks for step-up and the session has not just verified).
 * `Preview`, `OpError` and `Denied` count only when they answer a command sent from here.
 *
 * An enrolment token arrives once, in `HostEnrollment`, as the reply to that `Confirm`. It is held
 * here until dismissed (or the workspace changes) and never stored anywhere else.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import { usePasskeys } from '@/features/passkeys/store'
import type { Cmd, HostInfo, Preview, RequestId, SshCaInfo } from '@/protocol/wire'

/** The native input and select style the forms use. */
export const FIELD =
  'h-8 border border-input bg-background px-2 text-xs outline-none focus-visible:border-accent-neon'

const OFFLINE = 'Keel is not connected. Wait for it to reconnect, then try again.'

/** A token just minted: shown once. */
export interface Enrollment {
  id: string
  inventory_name: string
  profiles: string[]
  expires_at: string
  token: string
}

export interface SshCa {
  cas: SshCaInfo[]
  /** The `@cert-authority * …` line for `keel_runner_host_ca`. */
  cert_authority: string
}

/** The `ansible-playbook` extra var that hands a host its token. */
export function tokenArg(token: string): string {
  return `-e keel_enroll_token=${token}`
}

/** Splits "debian, web" or "debian web" into profile names. */
export function parseProfiles(text: string): string[] {
  return [...new Set(text.split(/[\s,]+/).filter((p) => p.length > 0))]
}

export interface HostsState {
  workspace: string | null
  hosts: HostInfo[]
  ssh: SshCa | null
  enrollment: Enrollment | null
  preview: Preview | null
  error: string | null
  load: (workspace: string) => void
  /** Previews a token for `inventory_name` with `profiles`. */
  enroll: (inventory_name: string, profiles: string[]) => void
  revoke: (host: string) => void
  confirm: () => Promise<void>
  cancel: () => void
  dismissEnrollment: () => void
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
  send({ type: 'ListHosts', data: { workspace, profile: null, drift: null } })
}

export const useHosts = create<HostsState>()((set, get) => ({
  workspace: null,
  hosts: [],
  ssh: null,
  enrollment: null,
  preview: null,
  error: null,

  load: (workspace) => {
    if (get().workspace !== workspace) set({ workspace, hosts: [], enrollment: null })
    list(workspace)
    send({ type: 'GetSshCa' })
  },
  enroll: (inventory_name, profiles) => {
    const workspace = get().workspace
    if (!workspace) return
    set({ error: null, preview: null })
    send({ type: 'PreviewHostEnroll', data: { workspace, inventory_name, profiles } })
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
  dismissEnrollment: () => {
    set({ enrollment: null })
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
    case 'Hosts':
      if (ours) useHosts.setState({ hosts: event.data.items })
      break
    case 'Host': {
      const { host } = event.data
      if (host.workspace === s.workspace) useHosts.setState({ hosts: upsert(s.hosts, host) })
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
    case 'HostEnrollment':
      if (ours && event.data.workspace === s.workspace) {
        const { id, inventory_name, profiles, expires_at, token } = event.data
        useHosts.setState({ enrollment: { id, inventory_name, profiles, expires_at, token }, preview: null })
      }
      break
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
