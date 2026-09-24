/**
 * Logs tab (Phase 3, US1): live-tails a pod's log output through the already-implemented
 * `StartPodLogs`/`StopPodLogs`/`PodLogsChunk`/`StreamClosed` backend plumbing.
 *
 * The backend's log stream is a singleton (one active tail app-wide, `state.log_stream`,
 * `src/backend/pods.rs::start_logs` stops any previous one before starting the next) with no
 * `stream_id` echoed back to the caller, so this component tracks "its" stream by remembering
 * the highest stream id already present when it sends `StartPodLogs` and watching for a newer
 * `kind: 'logs'` entry to appear (data-model.md "Log Stream").
 */
import { ArrowLineDownIcon, WarningIcon } from '@phosphor-icons/react'
import { useCallback, useEffect, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import type { PodDetails } from '@/protocol/types'
import type { ObjectRef } from '@/resources'
import { cn } from '@/lib/utils'
import { detailKey, useStore, type LogStreamState } from '@/store'

import { Picker } from '../Picker'
import { Loading } from '../shell/PaneStates'

/** How close to the bottom (px) still counts as "at the bottom" for auto-follow. */
const FOLLOW_THRESHOLD_PX = 24

function maxStreamId(streams: Record<number, unknown>): number {
  let max = 0
  for (const key of Object.keys(streams)) max = Math.max(max, Number(key))
  return max
}

export function PodLogsTab({ target }: { target: ObjectRef }) {
  const namespace = target.namespace ?? ''
  const name = target.name
  const send = useStore((s) => s.send)
  const context = useStore((s) => s.session.activeContext)
  const detailState = useStore((s) => s.details[detailKey(context, 'Pod', namespace, name)])
  const detail =
    detailState?.eventType === 'PodDetailsUpdated' ? (detailState.data as PodDetails) : undefined
  // 008-pod-lifecycle-diagnostics (FR-003): init containers' logs are reachable the same way as
  // app containers', so the picker lists both — init containers first, matching spec/status order.
  const containers = detail
    ? [
        ...detail.containers_lifecycle.filter((c) => c.is_init).map((c) => c.name),
        ...detail.containers,
        // 025-ephemeral-debug-containers: debug containers stream logs like any other container.
        ...detail.containers_lifecycle.filter((c) => c.is_ephemeral).map((c) => c.name),
      ]
    : []

  const podLogsIntent = useStore((s) => s.podLogsIntent)
  const clearPodLogsIntent = useStore((s) => s.clearPodLogsIntent)
  const [selected, setSelected] = useState<string | undefined>(undefined)

  // Consume a pending "view logs for this container" request (Lifecycle section) during render —
  // React's documented "adjusting state when a value changes" pattern (comparing against state,
  // not a ref, so it works with the React Compiler) — rather than in an effect. Clearing the
  // store's `podLogsIntent` is deferred to a microtask so it doesn't update a different store
  // subscriber mid-render.
  const [lastAppliedIntent, setLastAppliedIntent] = useState(podLogsIntent)
  if (podLogsIntent !== lastAppliedIntent) {
    setLastAppliedIntent(podLogsIntent)
    if (
      podLogsIntent &&
      podLogsIntent.namespace === namespace &&
      podLogsIntent.name === name &&
      containers.includes(podLogsIntent.container)
    ) {
      setSelected(podLogsIntent.container)
      queueMicrotask(clearPodLogsIntent)
    }
  }

  const activeContainer =
    containers.length > 0
      ? selected && containers.includes(selected)
        ? selected
        : containers[0]
      : undefined

  // The stream id "since" which a freshly appearing 'logs' entry belongs to this mount/container.
  const sinceIdRef = useRef(0)

  useEffect(() => {
    if (containers.length === 0) return // wait for PodDetails so a multi-container pod picks one
    sinceIdRef.current = maxStreamId(useStore.getState().streams)
    const container = containers.length > 1 ? (activeContainer ?? null) : null
    send({ type: 'StartPodLogs', data: { namespace, name, container } })
    return () => send({ type: 'StopPodLogs' })
  }, [namespace, name, activeContainer, containers.length, send])

  const stream = useStore((s) => {
    let bestId = -1
    let best: LogStreamState | undefined
    for (const key of Object.keys(s.streams)) {
      const id = Number(key)
      if (id <= sinceIdRef.current) continue
      const entry = s.streams[id]
      if (entry?.kind !== 'logs') continue
      if (id > bestId) {
        bestId = id
        best = entry
      }
    }
    return best
  })

  const lines = stream?.lines.toArray() ?? []
  const [follow, setFollow] = useState(true)
  const scrollRef = useRef<HTMLDivElement>(null)

  // Auto-scroll to the newest line whenever it changes, unless the operator scrolled up.
  useEffect(() => {
    if (!follow) return
    const el = scrollRef.current
    if (!el) return
    el.scrollTop = el.scrollHeight
  }, [follow, stream?.rev])

  const onScroll = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_THRESHOLD_PX
    setFollow(atBottom)
  }, [])

  const jumpToLatest = useCallback(() => {
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
    setFollow(true)
  }, [])

  const ended = stream !== undefined && stream.status !== 'open'

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {containers.length > 1 && (
        <div className="flex shrink-0 items-center gap-2 border-b border-border px-2 py-1.5">
          <Picker
            label="Container"
            prefix="container"
            value={activeContainer}
            options={containers.map((c) => ({ value: c, label: c }))}
            onChange={setSelected}
          />
        </div>
      )}
      <div className="relative min-h-0 flex-1">
        <div
          ref={scrollRef}
          onScroll={onScroll}
          data-testid="pod-logs-scroll"
          className="h-full min-w-0 overflow-y-auto bg-background/40 p-2 font-mono text-xs leading-relaxed"
        >
          {lines.length === 0 && !stream ? (
            <Loading rows={6} label="Starting log stream" />
          ) : (
            <>
              {(stream?.lines.truncated ?? 0) > 0 && (
                <p className="mb-1 text-xs text-muted-foreground">
                  … {stream?.lines.truncated} earlier line
                  {stream?.lines.truncated === 1 ? '' : 's'} truncated
                </p>
              )}
              <pre className="min-w-0 whitespace-pre-wrap break-words text-foreground/90">
                {lines.join('\n')}
              </pre>
            </>
          )}
        </div>
        {!follow && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={jumpToLatest}
            className={cn(
              'absolute right-3 bottom-3 border-accent-neon/60 bg-background/90 shadow-[0_0_16px_-4px_var(--accent-neon)]',
            )}
          >
            <ArrowLineDownIcon data-icon="inline-start" /> Jump to latest
          </Button>
        )}
      </div>
      {ended && (
        <div
          role="status"
          data-pane-state="stream-ended"
          className="flex shrink-0 items-center gap-2 border-t border-status-warn/40 bg-status-warn/10 px-3 py-1.5 text-xs text-status-warn"
        >
          <WarningIcon aria-hidden="true" className="size-4" />
          <span>Stream ended{stream.reason ? `: ${stream.reason}` : ''}</span>
        </div>
      )}
    </div>
  )
}
