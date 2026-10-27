/**
 * Runs (spec 055 US2, T072): the workspace's run list and the one run open, with its hosts and
 * its output.
 *
 * Lists follow what the server sends: `Runs`, `Run` and the pushed `RunState`. Opening a run sends
 * `GetRun` and `SubscribeRun{from_seq: 0}`; the reply is `RunSubscribed` and the stored output, then
 * output is pushed live as `RunEvent`/`RunLog`. Every line carries a seq, so after a reconnect the
 * store resubscribes from the last seq it holds and nothing is shown twice or skipped. A run that
 * changes state is re-read, so its host table and summary follow.
 *
 * Cancelling is two-step, like every change: `PreviewCancel` answers a `Preview`, shown in a
 * confirm dialog, then `Confirm`.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import type { Cmd, Preview, RequestId, RunHostInfo, RunInfo, RunPhase, RunReason } from '@/protocol/wire'

/** The native input and select style the forms use. */
export const FIELD =
  'h-8 border border-input bg-background px-2 text-xs outline-none focus-visible:border-accent-neon'

/** A commit, shortened for a table cell. */
export function short(commit: string | null): string {
  return commit ? commit.slice(0, 12) : '—'
}

const ACTIVE: readonly RunPhase[] = ['needs_review', 'pending_approval', 'queued', 'running']

/** A run that can still change: one that has not ended. */
export function active(state: RunPhase): boolean {
  return ACTIVE.includes(state)
}

/** `cancelled_by_user` as "cancelled by user". */
export function reasonText(reason: RunReason | null): string {
  return reason ? reason.replace(/_/g, ' ') : ''
}

const OFFLINE = 'Keel is not connected. Wait for it to reconnect, then try again.'
/** Most recent output lines kept on screen; older ones are counted as dropped. */
export const MAX_LINES = 10_000

export interface RunDetail {
  run: RunInfo
  hosts: RunHostInfo[]
}

export interface RunsState {
  workspace: string | null
  runs: RunInfo[]
  /** The run open, by id, once `Run` answers. */
  open: string | null
  detail: RunDetail | null
  /** Output lines of the open run; a partial last line is held in `partial`. */
  lines: string[]
  partial: string
  dropped: number
  /** Highest seq of the open run's output received. */
  seq: number
  preview: Preview | null
  error: string | null
  load: (workspace: string) => void
  /** Opens a run (or closes it with `null`): reads it and subscribes to its output. */
  watch: (run: string | null) => void
  propose: (cmd: Cmd) => void
  confirm: () => void
  cancel: () => void
  clearError: () => void
}

const mine = new Set<RequestId>()

function send(cmd: Cmd): boolean {
  const id = sendCmd(cmd)
  if (id === null) {
    useRuns.setState({ error: OFFLINE })
    return false
  }
  mine.add(id)
  return true
}

/** Appends `text` to the output, splitting it into lines. */
function append(s: RunsState, text: string): Pick<RunsState, 'lines' | 'partial' | 'dropped'> {
  const parts = (s.partial + text.replace(/\r\n/g, '\n')).split('\n')
  const partial = parts.pop() ?? ''
  let lines = parts.length > 0 ? [...s.lines, ...parts] : s.lines
  let dropped = s.dropped
  if (lines.length > MAX_LINES) {
    dropped += lines.length - MAX_LINES
    lines = lines.slice(lines.length - MAX_LINES)
  }
  return { lines, partial, dropped }
}

/** An event line for the log: what Ansible was doing, on which host. */
function eventLine(kind: string, host: string | null, task: string | null): string {
  const parts = [`[keel] ${kind}`]
  if (task) parts.push(task)
  if (host) parts.push(`(${host})`)
  return `${parts.join(' ')}\n`
}

export const useRuns = create<RunsState>()((set, get) => ({
  workspace: null,
  runs: [],
  open: null,
  detail: null,
  lines: [],
  partial: '',
  dropped: 0,
  seq: 0,
  preview: null,
  error: null,

  load: (workspace) => {
    if (get().workspace !== workspace) set({ workspace, runs: [] })
    send({
      type: 'ListRuns',
      data: { workspace, binding: null, host: null, actor: null, state: null, from: null, to: null },
    })
  },
  watch: (run) => {
    const was = get().open
    if (was === run) return
    if (was !== null) sendCmd({ type: 'UnsubscribeRun', data: { run: was } })
    set({ open: run, detail: null, lines: [], partial: '', dropped: 0, seq: 0, preview: null, error: null })
    if (run === null) return
    send({ type: 'GetRun', data: { run } })
    send({ type: 'SubscribeRun', data: { run, from_seq: 0 } })
  },
  propose: (cmd) => {
    set({ error: null, preview: null })
    send(cmd)
  },
  confirm: () => {
    const p = get().preview
    if (p) send({ type: 'Confirm', data: { preview_id: p.preview_id } })
  },
  cancel: () => {
    set({ preview: null })
  },
  clearError: () => {
    set({ error: null })
  },
}))

function upsert(items: RunInfo[], run: RunInfo): RunInfo[] {
  const i = items.findIndex((x) => x.id === run.id)
  if (i >= 0) return items.map((x) => (x.id === run.id ? run : x))
  return [run, ...items]
}

onServerEvent(({ reply_to, event }) => {
  const s = useRuns.getState()
  const ours = reply_to !== undefined && mine.has(reply_to)
  switch (event.type) {
    case 'Runs':
      if (ours) useRuns.setState({ runs: event.data.items })
      break
    case 'Run': {
      const { run, hosts } = event.data
      if (s.workspace !== null && run.workspace !== s.workspace && s.open !== run.id) break
      useRuns.setState({
        runs: s.workspace === run.workspace ? upsert(s.runs, run) : s.runs,
        ...(s.open === run.id ? { detail: { run, hosts } } : {}),
      })
      break
    }
    case 'RunState': {
      const { run, state, reason } = event.data
      useRuns.setState({
        runs: s.runs.map((r) => (r.id === run ? { ...r, state, reason } : r)),
        ...(s.detail?.run.id === run ? { detail: { ...s.detail, run: { ...s.detail.run, state, reason } } } : {}),
      })
      // Re-read the open run, so its hosts and summary follow its state.
      if (s.open === run) sendCmd({ type: 'GetRun', data: { run } })
      // A run this list has not seen yet: re-list.
      else if (s.workspace && !s.runs.some((r) => r.id === run)) {
        sendCmd({
          type: 'ListRuns',
          data: { workspace: s.workspace, binding: null, host: null, actor: null, state: null, from: null, to: null },
        })
      }
      break
    }
    // `RunSubscribed` names the head seq; the stored lines after `from_seq` follow it, each
    // carrying its own seq, so the store advances on the lines, not on this.
    case 'RunLog': {
      const { run, seq, text } = event.data
      if (run !== s.open || seq <= s.seq) break
      useRuns.setState({ seq, ...append(s, text) })
      break
    }
    case 'RunEvent': {
      const { run, seq, kind, host, task } = event.data
      if (run !== s.open || seq <= s.seq) break
      useRuns.setState({ seq, ...append(s, eventLine(kind, host, task)) })
      break
    }
    case 'Preview':
      if (ours) useRuns.setState({ preview: event.data, error: null })
      break
    case 'Confirmed':
      if (ours) useRuns.setState({ preview: null })
      break
    case 'OpError':
      if (ours) useRuns.setState({ error: event.data.message, preview: null })
      break
    case 'Denied':
      if (ours) useRuns.setState({ error: `Not allowed: ${event.data.reason}`, preview: null })
      break
    default:
      break
  }
})

onReconnected(() => {
  const { workspace, open, seq, load } = useRuns.getState()
  if (workspace) load(workspace)
  if (open) {
    send({ type: 'GetRun', data: { run: open } })
    send({ type: 'SubscribeRun', data: { run: open, from_seq: seq } })
  }
})
