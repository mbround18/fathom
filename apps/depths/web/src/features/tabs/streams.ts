/**
 * The live data behind the tabs (US4, T067): one log stream or shell session per tab, and one
 * stats watch shared by every visible stats tab. Fed by the shared socket, never holding it.
 *
 * - Every tab in the layout gets its stream; closing the tab sends `CloseStream`. A console (the
 *   stack page's log view, outside the layout) is a log stream like a tab's, for as long as it is
 *   registered with `openConsole`; `closeConsole` closes it the same way.
 * - Log lines are kept in a ring of `DEFAULT_RING_CAPACITY` lines, with control sequences removed
 *   (they are untrusted). The view shows "older lines dropped" from the ring's count.
 * - A log stream the proxy cut (`disconnected`) reopens with backoff and `since` the last line
 *   seen, and the tab says "Reconnected". A stream that `ended` (the container stopped) reopens
 *   when the stack list shows the target running again, and the tab says so.
 * - A dropped socket loses every stream: logs reopen with `since`; a shell cannot be resumed, so its
 *   tab says why it ended and offers a new session.
 * - Stats are watched only while some stats tab is on screen and the page is visible. The protocol
 *   has one watch per socket (`UnwatchStats` takes no target), so the union of targets is sent.
 */
import { create } from 'zustand'

import { RingBuffer, DEFAULT_RING_CAPACITY, type TerminalStream } from '@companion/terminal'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import { useStacks } from '@/features/stacks/store'
import type { LogTarget, RequestId, StatsTarget, StreamCloseReason, StreamId, UsageSample } from '@/protocol/generated'

import type { TabSpec } from './layout'
import { lineTime } from './logtext'
import { useTabs } from './store'

/** Lines asked for when a log tab first opens (and after a reload). */
export const LOG_TAIL = 1000
/** Output chunks kept for a shell, replayed when its tab is shown again. */
const SHELL_CHUNKS = 4096
const BACKOFF_MS = [500, 1000, 2000, 5000, 10_000] as const

export interface LogLine {
  /** The line as written, colour codes included; the log view renders it safely. */
  text: string
  /** When it was written (ms), from the line's own timestamp or the last one before it. */
  at: number | null
}

export type LogPhase = 'opening' | 'live' | 'ended' | 'failed' | 'denied'

export interface LogStream {
  phase: LogPhase
  lines: RingBuffer<LogLine>
  /** Bumped on every change to `lines`. */
  rev: number
  /** A passing note: "Reconnected", "Container restarted". */
  notice: string | null
  /** Why it ended, failed or was refused, in words. */
  reason: string | null
  endedAt: number | null
}

export interface ShellStream extends TerminalStream {
  /** Bumped for every new session in the same tab. */
  gen: number
  endedAt: number | null
  /** The session ended in a way the person should restart by hand. */
  offerNew: boolean
}

interface StreamsState {
  logs: Record<string, LogStream>
  shells: Record<string, ShellStream>
  /** The latest sample per container id. */
  samples: Record<string, UsageSample>
}

export const useStreams = create<StreamsState>()(() => ({ logs: {}, shells: {}, samples: {} }))

// --- Bookkeeping -----------------------------------------------------------------------------

interface Ref {
  tab: string
  gen: number
}

/** Open requests awaiting `StreamOpened`. */
const opening = new Map<RequestId, Ref>()
/** Open streams, both ways. */
const byStream = new Map<StreamId, Ref>()
const streamOf = new Map<string, StreamId>()
/** The current generation of each tab's stream; replies for an older one are closed unseen. */
const gens = new Map<string, number>()
/** Tabs this module has started. */
const known = new Set<string>()
/** Tabs whose open found no socket; retried when it opens. */
const unsent = new Set<string>()
/** Log resume points and notes to show once reopened. */
const lastStamp = new Map<string, { stamp: string; ms: number }>()
/**
 * The newest line before a resume `since` it. The server repeats lines at (or, with clock skew,
 * before) that instant; they are skipped until a newer line arrives. Only then: interleaved stack
 * logs are not in time order, and an older line after that point is a real line.
 */
const resumeFrom = new Map<string, { stamp: string; ms: number }>()
const pendingNotice = new Map<string, string>()
const attempts = new Map<string, number>()
const timers = new Map<string, ReturnType<typeof setTimeout>>()
/** Ended log tabs that have since been seen not running (so "running" again means restarted). */
const sawDown = new Set<string>()
/** Each shell's size and UTF-8 decoder. */
const sizes = new Map<string, { cols: number; rows: number }>()
const decoders = new Map<string, TextDecoder>()

function bumpGen(tab: string): number {
  const g = (gens.get(tab) ?? 0) + 1
  gens.set(tab, g)
  return g
}

function forgetStream(tab: string): void {
  const id = streamOf.get(tab)
  if (id === undefined) return
  streamOf.delete(tab)
  byStream.delete(id)
}

type LogSpec = Extract<TabSpec, { kind: 'logs' }>

/** Log streams outside the tab layout, by id: the stack page's console. */
const consoles = new Map<string, LogSpec>()

/** Every stream this module should have open: the layout's tabs, then the consoles. */
function specs(): TabSpec[] {
  return [...useTabs.getState().layout.tabs, ...consoles.values()]
}

function tabSpec(id: string): TabSpec | undefined {
  return consoles.get(id) ?? useTabs.getState().layout.tabs.find((t) => t.id === id)
}

function setLog(tab: string, patch: Partial<LogStream>): void {
  useStreams.setState((s) => {
    const cur = s.logs[tab] ?? emptyLog()
    return { logs: { ...s.logs, [tab]: { ...cur, ...patch } } }
  })
}

function setShell(tab: string, patch: Partial<ShellStream>): void {
  useStreams.setState((s) => {
    const cur = s.shells[tab]
    if (!cur) return s
    return { shells: { ...s.shells, [tab]: { ...cur, ...patch } } }
  })
}

function emptyLog(): LogStream {
  return {
    phase: 'opening',
    lines: new RingBuffer<LogLine>(DEFAULT_RING_CAPACITY),
    rev: 0,
    notice: null,
    reason: null,
    endedAt: null,
  }
}

// --- Logs ------------------------------------------------------------------------------------

function openLogs(tab: Extract<TabSpec, { kind: 'logs' }>, resume: boolean): void {
  clearTimer(tab.id)
  forgetStream(tab.id)
  const gen = bumpGen(tab.id)
  const from = resume ? lastStamp.get(tab.id) : undefined
  const since = from?.stamp
  if (from) resumeFrom.set(tab.id, from)
  else resumeFrom.delete(tab.id)
  const id = sendCmd({
    type: 'OpenLogs',
    data: since === undefined ? { target: tab.target, tail: LOG_TAIL } : { target: tab.target, tail: LOG_TAIL, since },
  })
  if (!useStreams.getState().logs[tab.id]) setLog(tab.id, emptyLog())
  setLog(tab.id, { phase: 'opening', reason: null })
  if (id === null) unsent.add(tab.id)
  else {
    unsent.delete(tab.id)
    opening.set(id, { tab: tab.id, gen })
  }
}

function appendLines(tab: string, raw: readonly string[]): void {
  const cur = useStreams.getState().logs[tab]
  if (!cur) return
  let last = lastStamp.get(tab)
  let resume = resumeFrom.get(tab)
  let at = last?.ms ?? null
  const out: LogLine[] = []
  for (const r of raw) {
    const t = lineTime(r)
    if (t) {
      if (resume) {
        if (t.ms < resume.ms) continue
        if (t.ms === resume.ms && repeats(cur.lines, r)) continue
        if (t.ms > resume.ms) {
          resumeFrom.delete(tab)
          resume = undefined
        }
      }
      at = t.ms
      if (!last || t.ms >= last.ms) {
        last = t
        lastStamp.set(tab, t)
      }
    }
    out.push({ text: r, at })
  }
  if (out.length === 0) return
  cur.lines.pushAll(out)
  setLog(tab, { rev: cur.rev + 1 })
}

/** The line is already among the retained lines written at the same instant. */
function repeats(lines: RingBuffer<LogLine>, raw: string): boolean {
  const last = lines.at(lines.length - 1)?.at
  for (let i = lines.length - 1; i >= 0 && i >= lines.length - 200; i--) {
    const l = lines.at(i)
    if (!l || l.at !== last) break
    if (l.text === raw) return true
  }
  return false
}

function clearTimer(tab: string): void {
  const t = timers.get(tab)
  if (t !== undefined) clearTimeout(t)
  timers.delete(tab)
}

function reopenLater(tab: string, notice: string): void {
  clearTimer(tab)
  const n = attempts.get(tab) ?? 0
  attempts.set(tab, n + 1)
  pendingNotice.set(tab, notice)
  timers.set(
    tab,
    setTimeout(() => {
      timers.delete(tab)
      const spec = tabSpec(tab)
      if (spec?.kind === 'logs') openLogs(spec, true)
    }, BACKOFF_MS[Math.min(n, BACKOFF_MS.length - 1)]),
  )
}

/** Whether the stack list shows a log target running; `null` when it cannot tell. */
export function targetRunning(target: LogTarget): boolean | null {
  const list = useStacks.getState().list
  if (!list) return null
  if (target.kind === 'stack') {
    const stack = list.stacks.find((s) => s.name === target.stack)
    if (!stack) return null
    if (target.service === undefined) return stack.status === 'running' || stack.status === 'partial'
    const svc = stack.services.find((s) => s.name === target.service)
    return svc ? svc.containers.some((c) => c.state === 'running') : null
  }
  const all = [
    ...list.stacks.flatMap((s) => s.services.flatMap((v) => v.containers)),
    ...list.foreign.flatMap((f) => f.containers),
    ...list.standalone,
  ]
  const c = all.find((x) => x.name === target.container || x.id === target.container || x.id.startsWith(target.container))
  return c ? c.state === 'running' : null
}

function checkRestarts(): void {
  for (const [tab, log] of Object.entries(useStreams.getState().logs)) {
    if (log.phase !== 'ended') continue
    const spec = tabSpec(tab)
    if (spec?.kind !== 'logs') continue
    const running = targetRunning(spec.target)
    if (running === false) sawDown.add(tab)
    else if (running === true && sawDown.has(tab)) {
      sawDown.delete(tab)
      pendingNotice.set(tab, 'Container restarted')
      openLogs(spec, true)
    }
  }
}

// --- Shells ----------------------------------------------------------------------------------

const DEFAULT_SIZE = { cols: 80, rows: 24 }

function openShell(tab: Extract<TabSpec, { kind: 'shell' }>): void {
  forgetStream(tab.id)
  const gen = bumpGen(tab.id)
  decoders.set(tab.id, new TextDecoder())
  const stream: ShellStream = {
    status: 'attaching',
    chunks: new RingBuffer<string>(SHELL_CHUNKS),
    rev: 0,
    gen,
    endedAt: null,
    offerNew: false,
  }
  useStreams.setState((s) => ({ shells: { ...s.shells, [tab.id]: stream } }))
  const { cols, rows } = sizes.get(tab.id) ?? DEFAULT_SIZE
  const id = sendCmd({
    type: 'OpenShell',
    data: tab.shell ? { container: tab.container, shell: tab.shell, cols, rows } : { container: tab.container, cols, rows },
  })
  if (id === null) unsent.add(tab.id)
  else {
    unsent.delete(tab.id)
    opening.set(id, { tab: tab.id, gen })
  }
}

/** A shell tab that shows why it ended and offers a new session, without starting one. */
function offerShell(tab: string, reason: string, endedAt: number | null): void {
  const cur = useStreams.getState().shells[tab]
  const stream: ShellStream = {
    status: 'ended',
    reason,
    chunks: cur?.chunks ?? new RingBuffer<string>(SHELL_CHUNKS),
    rev: cur?.rev ?? 0,
    gen: cur?.gen ?? gens.get(tab) ?? 0,
    endedAt,
    offerNew: true,
  }
  useStreams.setState((s) => ({ shells: { ...s.shells, [tab]: stream } }))
}

/** Starts a new session in a shell tab (the "New session" button). */
export function newShellSession(tab: string): void {
  const id = streamOf.get(tab)
  if (id !== undefined) sendCmd({ type: 'CloseStream', data: { stream_id: id } })
  useTabs.getState().updateTab(tab, (t) => (t.kind === 'shell' ? { ...t, offerNew: false } : t))
  const spec = tabSpec(tab)
  if (spec?.kind === 'shell') openShell(spec)
}

export function shellInput(tab: string, bytesB64: string): void {
  const id = streamOf.get(tab)
  if (id !== undefined && useStreams.getState().shells[tab]?.status === 'active') {
    sendCmd({ type: 'ShellInput', data: { stream_id: id, bytes_b64: bytesB64 } })
  }
}

export function shellResize(tab: string, cols: number, rows: number): void {
  // A hidden or collapsed terminal fits to nothing; keep the last real size.
  if (cols < 2 || rows < 2) return
  const prev = sizes.get(tab)
  if (prev?.cols === cols && prev.rows === rows) return
  sizes.set(tab, { cols, rows })
  const id = streamOf.get(tab)
  if (id !== undefined) sendCmd({ type: 'ShellResize', data: { stream_id: id, cols, rows } })
}

const SHELL_END: Record<StreamCloseReason['kind'], string> = {
  ended: 'The shell exited.',
  closed: 'The session was closed.',
  disconnected: 'The connection to the container dropped.',
  failed: 'The shell failed.',
  container_recreated: 'The container was recreated, so its shell was closed.',
  container_removed: 'The container is gone, so its shell was closed.',
}

function decode(b64: string, decoder: TextDecoder | undefined): string {
  let bin: string
  try {
    bin = atob(b64)
  } catch {
    return ''
  }
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return (decoder ?? new TextDecoder()).decode(bytes, { stream: true })
}

// --- Stats -----------------------------------------------------------------------------------

const watchers = new Map<string, StatsTarget>()
let watching = ''

function pageVisible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden'
}

function syncStats(): void {
  const seen = new Map<string, StatsTarget>()
  if (pageVisible()) for (const t of watchers.values()) seen.set(JSON.stringify(t), t)
  const targets = [...seen.values()]
  const key = [...seen.keys()].sort().join('\n')
  if (key === watching) return
  if (watching !== '') sendCmd({ type: 'UnwatchStats' })
  if (targets.length > 0) sendCmd({ type: 'WatchStats', data: { targets } })
  watching = key
}

/** A stats tab is on screen: watch its target until the returned function is called. */
export function watchStats(tab: string, target: StatsTarget): () => void {
  return watchStatsAll([[tab, target]])
}

/** `watchStats` for several keys at once, sending one `WatchStats` rather than one per key. */
export function watchStatsAll(entries: readonly (readonly [string, StatsTarget])[]): () => void {
  for (const [key, target] of entries) watchers.set(key, target)
  syncStats()
  return () => {
    for (const [key] of entries) watchers.delete(key)
    syncStats()
  }
}

if (typeof document !== 'undefined') document.addEventListener('visibilitychange', syncStats)

// --- Reconciling with the layout --------------------------------------------------------------

function start(tab: TabSpec): void {
  if (tab.kind === 'logs') openLogs(tab, lastStamp.has(tab.id))
  else if (tab.kind === 'shell') {
    if (tab.offerNew) offerShell(tab.id, 'Your session expired.', null)
    else openShell(tab)
  }
}

function drop(tab: string): void {
  const id = streamOf.get(tab)
  if (id !== undefined) sendCmd({ type: 'CloseStream', data: { stream_id: id } })
  forgetStream(tab)
  clearTimer(tab)
  for (const m of [gens, lastStamp, resumeFrom, pendingNotice, attempts, sizes, decoders]) m.delete(tab)
  known.delete(tab)
  unsent.delete(tab)
  sawDown.delete(tab)
  useStreams.setState((s) => ({
    logs: Object.fromEntries(Object.entries(s.logs).filter(([k]) => k !== tab)),
    shells: Object.fromEntries(Object.entries(s.shells).filter(([k]) => k !== tab)),
  }))
}

/** Forget everything without sending: the socket (and every stream on it) is gone. */
export function resetStreams(): void {
  for (const t of timers.values()) clearTimeout(t)
  for (const m of [opening, byStream, streamOf, gens, lastStamp, resumeFrom, pendingNotice, attempts, timers, sizes, decoders])
    m.clear()
  known.clear()
  unsent.clear()
  sawDown.clear()
  watchers.clear()
  watching = ''
  useStreams.setState({ logs: {}, shells: {}, samples: {} })
}

function reconcile(): void {
  const { layout, subject } = useTabs.getState()
  if (subject === null) {
    if (known.size > 0 || watching !== '') resetStreams()
    return
  }
  const all = [...layout.tabs, ...consoles.values()]
  const ids = new Set(all.map((t) => t.id))
  for (const tab of [...known]) if (!ids.has(tab)) drop(tab)
  for (const tab of all) {
    if (known.has(tab.id)) continue
    known.add(tab.id)
    start(tab)
  }
}

function retryUnsent(): void {
  for (const tab of [...unsent]) {
    const spec = tabSpec(tab)
    if (spec) start(spec)
  }
}

useTabs.subscribe((s, prev) => {
  if (s.layout.tabs !== prev.layout.tabs || s.subject !== prev.subject) reconcile()
})

useSession.subscribe((s, prev) => {
  if (s.connection === 'open' && prev.connection !== 'open') retryUnsent()
})

useStacks.subscribe((s, prev) => {
  if (s.list !== prev.list) checkRestarts()
})

onReconnected(() => {
  // Every stream lived on the old socket. Requests queued while it was down still go out, but
  // their generation is stale now, so their streams are closed on arrival.
  opening.clear()
  byStream.clear()
  streamOf.clear()
  const { logs, shells } = useStreams.getState()
  for (const tab of specs()) {
    if (tab.kind === 'logs' && logs[tab.id]) {
      pendingNotice.set(tab.id, 'Reconnected')
      openLogs(tab, true)
    } else if (tab.kind === 'shell' && shells[tab.id] && !shells[tab.id]?.offerNew) {
      bumpGen(tab.id)
      offerShell(tab.id, 'The connection to Depths dropped.', Date.now())
    }
  }
  watching = ''
  syncStats()
})

onServerEvent(({ reply_to, event }) => {
  const asked = reply_to === undefined ? undefined : opening.get(reply_to)
  if (reply_to !== undefined && asked) opening.delete(reply_to)

  switch (event.type) {
    case 'StreamOpened': {
      if (!asked) return
      const id = event.data.stream_id
      if (gens.get(asked.tab) !== asked.gen || !known.has(asked.tab)) {
        sendCmd({ type: 'CloseStream', data: { stream_id: id } })
        return
      }
      byStream.set(id, asked)
      streamOf.set(asked.tab, id)
      attempts.delete(asked.tab)
      const logs = useStreams.getState().logs
      if (logs[asked.tab]) {
        const notice = pendingNotice.get(asked.tab) ?? null
        pendingNotice.delete(asked.tab)
        setLog(asked.tab, { phase: 'live', notice, reason: null, endedAt: null })
      } else {
        setShell(asked.tab, { status: 'active' })
      }
      return
    }
    case 'Denied':
    case 'Invalid': {
      if (!asked || gens.get(asked.tab) !== asked.gen) return
      const reason =
        event.type === 'Invalid'
          ? event.data.reason
          : event.data.reason === 'feature_disabled'
            ? 'The shell is turned off on this server.'
            : 'You are not permitted to see this.'
      if (useStreams.getState().logs[asked.tab]) setLog(asked.tab, { phase: 'denied', reason })
      else setShell(asked.tab, { status: 'error', reason, endedAt: Date.now() })
      return
    }
    case 'LogChunk': {
      const ref = byStream.get(event.data.stream_id)
      if (ref) appendLines(ref.tab, event.data.lines)
      return
    }
    case 'ShellOutput': {
      const ref = byStream.get(event.data.stream_id)
      const cur = ref && useStreams.getState().shells[ref.tab]
      if (!ref || !cur) return
      const text = decode(event.data.bytes_b64, decoders.get(ref.tab))
      if (text === '') return
      cur.chunks.push(text)
      setShell(ref.tab, { rev: cur.rev + 1 })
      return
    }
    case 'StreamClosed': {
      const ref = byStream.get(event.data.stream_id)
      if (!ref) return
      byStream.delete(event.data.stream_id)
      if (streamOf.get(ref.tab) === event.data.stream_id) streamOf.delete(ref.tab)
      onClosed(ref.tab, event.data.reason)
      return
    }
    case 'StatsSample':
      useStreams.setState((s) => {
        const samples = { ...s.samples }
        for (const x of event.data.samples) samples[x.container_id] = x
        return { samples }
      })
      return
    default:
      return
  }
})

function onClosed(tab: string, reason: StreamCloseReason): void {
  const spec = tabSpec(tab)
  if (!spec) return
  const now = Date.now()
  if (spec.kind === 'logs') {
    switch (reason.kind) {
      case 'disconnected':
        reopenLater(tab, 'Reconnected')
        return
      case 'ended':
        setLog(tab, { phase: 'ended', reason: 'The container stopped.', endedAt: now, notice: null })
        if (targetRunning(spec.target) !== true) sawDown.add(tab)
        return
      case 'closed':
        setLog(tab, { phase: 'ended', reason: 'The stream was closed.', endedAt: now, notice: null })
        return
      case 'failed':
        setLog(tab, { phase: 'failed', reason: reason.detail, endedAt: now, notice: null })
        return
    }
  }
  if (spec.kind === 'shell') {
    const why = reason.kind === 'failed' ? `${SHELL_END.failed} ${reason.detail}` : SHELL_END[reason.kind]
    setShell(tab, { status: reason.kind === 'failed' ? 'error' : 'ended', reason: why, endedAt: now, offerNew: true })
  }
}

/**
 * Streams `target`'s logs under `id` (read them with `useStreams((s) => s.logs[id])`) until
 * `closeConsole(id)`. The id must not be a tab's: `console:` ids are never made by the layout.
 */
export function openConsole(id: string, target: LogTarget): void {
  const cur = consoles.get(id)
  if (cur && JSON.stringify(cur.target) === JSON.stringify(target)) return
  if (cur) {
    consoles.delete(id)
    reconcile()
  }
  consoles.set(id, { id, kind: 'logs', target })
  reconcile()
}

/** Stops a console's stream (`CloseStream`) and forgets its lines. */
export function closeConsole(id: string): void {
  if (consoles.delete(id)) reconcile()
}

/** Retry a failed log tab. */
export function retryLogs(tab: string): void {
  const spec = tabSpec(tab)
  if (spec?.kind === 'logs') openLogs(spec, true)
}

reconcile()
