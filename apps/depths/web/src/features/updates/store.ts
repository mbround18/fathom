/**
 * The Updates page (spec 062 US1, T041) over the shared socket.
 *
 * - `load()` sends `ListUpdates`; the reply is `Updates`, then the server pushes `UpdatesChanged`
 *   (coalesced, at most one a second) for as long as the socket lives.
 * - `checkNow()` sends `CheckUpdatesNow`: `CheckStarted`, `CheckBusy{retry_after_secs}`, or
 *   `Invalid` while checks are off.
 * - `previewSettings(s)` sends `PreviewUpdateChange(settings)`; the reply is a `Preview` whose
 *   `file_diff` is the before/after summary. `confirm()` sends its `preview_id`.
 * - US1 is read-only otherwise: no batch, no policy, no Auto.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { Preview, RequestId, UpdateSettings, UpdatesView } from '@/protocol/generated'

import { DENY_TEXT } from '@/features/stacks/argv'

import type { StateFilter } from './model'

export type CheckStatus =
  | { kind: 'idle' }
  | { kind: 'sending' }
  | { kind: 'started' }
  | { kind: 'busy'; retryAfterSecs: number }
  | { kind: 'refused'; reason: string }

export type SettingsFlow =
  | { kind: 'idle' }
  | { kind: 'previewing'; after: UpdateSettings }
  | { kind: 'preview'; after: UpdateSettings; preview: Preview }
  | { kind: 'confirming'; after: UpdateSettings; preview: Preview }
  | { kind: 'refused'; reason: string }

export interface UpdatesState {
  view: UpdatesView | null
  loaded: boolean
  /** `Denied`: not an admin, or the feature is off. */
  denied: string | null
  stateFilter: StateFilter
  stackFilter: string
  check: CheckStatus
  settings: SettingsFlow
  load: () => void
  setStateFilter: (f: StateFilter) => void
  setStackFilter: (s: string) => void
  checkNow: () => void
  previewSettings: (after: UpdateSettings) => void
  confirm: () => void
  cancel: () => void
}

type Asked = 'list' | 'check' | 'preview' | 'confirm'
const pending = new Map<RequestId, Asked>()
let wanted = false

const INITIAL = {
  view: null,
  loaded: false,
  denied: null,
  stateFilter: 'all' as StateFilter,
  stackFilter: '',
  check: { kind: 'idle' } as CheckStatus,
  settings: { kind: 'idle' } as SettingsFlow,
}

function send(asked: Asked, cmd: Parameters<typeof sendCmd>[0]): boolean {
  const id = sendCmd(cmd)
  if (id === null) return false
  pending.set(id, asked)
  return true
}

export const useUpdates = create<UpdatesState>()((set, get) => ({
  ...INITIAL,

  load: () => {
    wanted = true
    send('list', { type: 'ListUpdates' })
  },

  setStateFilter: (stateFilter) => set({ stateFilter }),
  setStackFilter: (stackFilter) => set({ stackFilter }),

  checkNow: () => {
    if (get().check.kind === 'sending') return
    set({ check: send('check', { type: 'CheckUpdatesNow', data: {} }) ? { kind: 'sending' } : { kind: 'idle' } })
  },

  previewSettings: (after) => {
    const ok = send('preview', { type: 'PreviewUpdateChange', data: { kind: 'settings', data: after } })
    set({ settings: ok ? { kind: 'previewing', after } : { kind: 'refused', reason: 'Not connected.' } })
  },

  confirm: () => {
    const flow = get().settings
    if (flow.kind !== 'preview') return
    const ok = send('confirm', { type: 'Confirm', data: { preview_id: flow.preview.preview_id } })
    set({ settings: ok ? { ...flow, kind: 'confirming' } : { kind: 'refused', reason: 'Not connected.' } })
  },

  cancel: () => set({ settings: { kind: 'idle' } }),
}))

function voidText(reason: string): string {
  switch (reason) {
    case 'expired':
      return 'The preview expired. Try again.'
    case 'used':
      return 'That preview was already used.'
    default:
      return 'The preview is no longer valid. Try again.'
  }
}

onServerEvent(({ reply_to, event }) => {
  // Pushed changes carry no request id.
  if (event.type === 'UpdatesChanged') {
    useUpdates.setState({ view: event.data, loaded: true })
  }
  if (reply_to === undefined) return
  const asked = pending.get(reply_to)
  if (!asked) return
  pending.delete(reply_to)
  const s = useUpdates.getState()

  switch (asked) {
    case 'list':
      if (event.type === 'Updates') useUpdates.setState({ view: event.data, loaded: true, denied: null })
      else if (event.type === 'Denied') useUpdates.setState({ loaded: true, denied: event.data.reason })
      return
    case 'check':
      if (event.type === 'CheckStarted') useUpdates.setState({ check: { kind: 'started' } })
      else if (event.type === 'CheckBusy')
        useUpdates.setState({ check: { kind: 'busy', retryAfterSecs: event.data.retry_after_secs } })
      else if (event.type === 'Invalid') useUpdates.setState({ check: { kind: 'refused', reason: event.data.reason } })
      else if (event.type === 'Denied')
        useUpdates.setState({ check: { kind: 'refused', reason: DENY_TEXT[event.data.reason] } })
      else useUpdates.setState({ check: { kind: 'idle' } })
      return
    case 'preview':
      if (s.settings.kind !== 'previewing') return
      if (event.type === 'Preview')
        useUpdates.setState({ settings: { kind: 'preview', after: s.settings.after, preview: event.data } })
      else if (event.type === 'Invalid') useUpdates.setState({ settings: { kind: 'refused', reason: event.data.reason } })
      else if (event.type === 'Denied')
        useUpdates.setState({
          settings: { kind: 'refused', reason: DENY_TEXT[event.data.reason] },
        })
      return
    case 'confirm':
      if (event.type === 'UpdatesChanged') useUpdates.setState({ settings: { kind: 'idle' } })
      else if (event.type === 'PreviewVoid')
        useUpdates.setState({ settings: { kind: 'refused', reason: voidText(event.data.reason) } })
      else if (event.type === 'Invalid') useUpdates.setState({ settings: { kind: 'refused', reason: event.data.reason } })
      else if (event.type === 'Denied')
        useUpdates.setState({
          settings: { kind: 'refused', reason: DENY_TEXT[event.data.reason] },
        })
      return
  }
})

onReconnected(() => {
  pending.clear()
  useUpdates.setState({ check: { kind: 'idle' }, settings: { kind: 'idle' } })
  if (wanted) useUpdates.getState().load()
})

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') {
    resetUpdatesStore()
    return
  }
  if (s.connection === 'open' && prev.connection !== 'open' && wanted && !useUpdates.getState().loaded) {
    useUpdates.getState().load()
  }
})

/** Test hook. */
export function resetUpdatesStore(): void {
  pending.clear()
  wanted = false
  useUpdates.setState({ ...INITIAL })
}
