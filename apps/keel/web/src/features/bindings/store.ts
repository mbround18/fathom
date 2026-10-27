/**
 * Bindings, receivers, deliveries and credentials (spec 055 US1, T052). One store for both pages:
 * the receivers page binds receivers to the bindings listed here.
 *
 * Lists follow whatever the server sends (replies and pushes alike): `Bindings`, `Binding`,
 * `Receivers`, `Deliveries`, `DeliveryReceived`, `Credentials`. Every change is two-step: a
 * `Preview*` command answers a `Preview`, shown in a confirm dialog, then `Confirm` (with a
 * passkey assertion when the preview asks for step-up and the session has not just verified).
 * `Preview`, `OpError` and `Denied` count only when they answer a command sent from here.
 *
 * A receiver's secret arrives once, in `ReceiverSecret`; it is held until dismissed and never
 * stored anywhere else.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import { usePasskeys } from '@/features/passkeys/store'
import type {
  BindingInfo,
  Cmd,
  CredentialInfo,
  DeliveryInfo,
  JobSummary,
  Preview,
  ReceiverInfo,
  RequestId,
} from '@/protocol/wire'

/** The native input and select style the forms use. */
export const FIELD =
  'h-8 border border-input bg-background px-2 text-xs outline-none focus-visible:border-accent-neon'

/** A commit, shortened for a table cell. */
export function short(commit: string | null): string {
  return commit ? commit.slice(0, 12) : '—'
}

const OFFLINE = 'Keel is not connected. Wait for it to reconnect, then try again.'
const DELIVERY_LIMIT = 200

export interface ReposState {
  workspace: string | null
  bindings: BindingInfo[]
  detail: { binding: BindingInfo; jobs: JobSummary[] } | null
  receivers: ReceiverInfo[]
  deliveries: DeliveryInfo[]
  credentials: CredentialInfo[]
  preview: Preview | null
  /** A receiver's secret, shown once. */
  secret: { url: string; secret: string } | null
  error: string | null
  /** Picks the workspace and loads its lists. */
  load: (workspace: string) => void
  open: (binding: string | null) => void
  /** Sends a `Preview*` command; its `Preview` opens the confirm dialog. */
  propose: (cmd: Cmd) => void
  confirm: () => Promise<void>
  cancel: () => void
  sync: (binding: string) => void
  dismissSecret: () => void
  clearError: () => void
}

/** Commands sent from here, so their errors and previews land here. */
const mine = new Set<RequestId>()

function send(cmd: Cmd): boolean {
  const id = sendCmd(cmd)
  if (id === null) {
    useRepos.setState({ error: OFFLINE })
    return false
  }
  mine.add(id)
  return true
}

function upsert<T extends { id: string }>(items: T[], item: T, front = false): T[] {
  const i = items.findIndex((x) => x.id === item.id)
  if (i >= 0) return items.map((x) => (x.id === item.id ? item : x))
  return front ? [item, ...items] : [...items, item]
}

export const useRepos = create<ReposState>()((set, get) => ({
  workspace: null,
  bindings: [],
  detail: null,
  receivers: [],
  deliveries: [],
  credentials: [],
  preview: null,
  secret: null,
  error: null,

  load: (workspace) => {
    if (get().workspace !== workspace) {
      set({ workspace, bindings: [], detail: null, receivers: [], deliveries: [], credentials: [] })
    }
    send({ type: 'ListBindings', data: { workspace } })
    send({ type: 'ListReceivers', data: { workspace } })
    send({ type: 'ListCredentials', data: { workspace } })
    send({
      type: 'ListDeliveries',
      data: { workspace, receiver: null, binding: null, decision: null, from: null, to: null },
    })
  },
  open: (binding) => {
    if (binding === null) set({ detail: null })
    else send({ type: 'GetBinding', data: { binding } })
  },
  propose: (cmd) => {
    set({ error: null, preview: null })
    send(cmd)
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
  sync: (binding) => {
    set({ error: null })
    send({ type: 'SyncBinding', data: { binding } })
  },
  dismissSecret: () => {
    set({ secret: null })
  },
  clearError: () => {
    set({ error: null })
  },
}))

onServerEvent(({ reply_to, event }) => {
  const s = useRepos.getState()
  const ours = reply_to !== undefined && mine.has(reply_to)
  switch (event.type) {
    case 'Bindings':
      useRepos.setState({ bindings: event.data.items })
      break
    case 'Binding': {
      const { binding, jobs } = event.data
      if (s.workspace !== null && binding.workspace !== s.workspace) break
      const showing = ours || s.detail?.binding.id === binding.id
      useRepos.setState({
        bindings: upsert(s.bindings, binding),
        ...(showing ? { detail: { binding, jobs } } : {}),
      })
      break
    }
    case 'BindingSynced':
      // Refresh what changed: the list always, the detail when it is the one open.
      if (s.workspace) sendCmd({ type: 'ListBindings', data: { workspace: s.workspace } })
      if (s.detail?.binding.id === event.data.binding) {
        sendCmd({ type: 'GetBinding', data: { binding: event.data.binding } })
      }
      break
    case 'Receivers':
      useRepos.setState({ receivers: event.data.items })
      break
    case 'ReceiverSecret':
      if (ours) useRepos.setState({ secret: event.data })
      break
    case 'Deliveries':
      useRepos.setState({ deliveries: event.data.items })
      break
    case 'DeliveryReceived':
      useRepos.setState({
        deliveries: upsert(s.deliveries, event.data.delivery, true).slice(0, DELIVERY_LIMIT),
      })
      break
    case 'Credentials':
      useRepos.setState({ credentials: event.data.items })
      break
    case 'Preview':
      if (ours) useRepos.setState({ preview: event.data, error: null })
      break
    case 'Confirmed':
      if (ours) useRepos.setState({ preview: null })
      break
    case 'OpError':
      if (ours) useRepos.setState({ error: event.data.message, preview: null })
      break
    case 'Denied':
      if (ours) useRepos.setState({ error: `Not allowed: ${event.data.reason}`, preview: null })
      break
    default:
      break
  }
})

onReconnected(() => {
  const ws = useRepos.getState().workspace
  if (ws) useRepos.getState().load(ws)
})
