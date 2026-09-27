/**
 * Logs tab (Phase 3, US1): live-tails a pod's log output.
 *
 * Every mounted logs view (inspector Logs tab, dock Logs tool, tools sheet) owns its own backend
 * stream under a client-chosen id (`startPodLogs`/`stopPodLogs`, feedback #2), so views never
 * stop each other's streams. The stream is found again by this view's `owner` tag. States:
 * starting (skeleton), streaming (lines, or "No log output yet" for a quiet container), closed,
 * and error (inline message with Retry). A failed pod-details read is an inline error too.
 */
import { ArrowClockwiseIcon, ArrowLineDownIcon, WarningIcon } from '@phosphor-icons/react'
import { useCallback, useEffect, useId, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import type { PodDetails } from '@/protocol/types'
import type { ObjectRef } from '@/resources'
import { cn } from '@/lib/utils'
import { detailKey, useStore, type LogStreamState, type StoreState } from '@/store'
import { waitForCommandResult } from '@/store/actions'

import { Picker } from '../Picker'
import { ErrorState, Loading } from '../shell/PaneStates'

/** How close to the bottom (px) still counts as "at the bottom" for auto-follow. */
const FOLLOW_THRESHOLD_PX = 24

/** This view's live stream: the (single) log stream tagged with its owner. */
function ownedStream(streams: StoreState['streams'], owner: string): LogStreamState | undefined {
  for (const entry of Object.values(streams)) {
    if (entry.kind === 'logs' && entry.owner === owner) return entry
  }
  return undefined
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

/**
 * Reads the pod's details when the store has none yet (the containers list is needed to pick a
 * container), so a failed read shows an error here rather than a skeleton forever.
 */
function usePodDetailsRead(namespace: string, name: string, hasDetail: boolean) {
  const request = useStore((s) => s.request)
  const [error, setError] = useState<string | undefined>(undefined)
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    if (hasDetail) return
    let cancelled = false
    request({ type: 'SelectPod', data: { namespace, name } })
      .then((id) => waitForCommandResult(id))
      .then((result) => {
        if (cancelled) return
        if (result === 'timeout') setError('Timed out reading the pod.')
        else if (!result.ok) setError(result.error ?? 'Could not read the pod.')
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(errorText(err))
      })
    return () => {
      cancelled = true
    }
  }, [request, namespace, name, hasDetail, attempt])

  const retry = useCallback(() => {
    setError(undefined)
    setAttempt((n) => n + 1)
  }, [])
  return { error, retry }
}

export function PodLogsTab({ target }: { target: ObjectRef }) {
  const namespace = target.namespace ?? ''
  const name = target.name
  const context = useStore((s) => s.session.activeContext)
  const startPodLogs = useStore((s) => s.startPodLogs)
  const stopPodLogs = useStore((s) => s.stopPodLogs)
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
  const detailsRead = usePodDetailsRead(namespace, name, detail !== undefined)

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

  const owner = useId()
  // Bumped by Retry/Restart: restarts the stream under a fresh id.
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    if (containers.length === 0) return // wait for PodDetails so a multi-container pod picks one
    const container = containers.length > 1 ? (activeContainer ?? null) : null
    const id = startPodLogs({ namespace, name, container }, owner)
    return () => stopPodLogs(id)
  }, [
    namespace,
    name,
    activeContainer,
    containers.length,
    startPodLogs,
    stopPodLogs,
    owner,
    attempt,
  ])

  const stream = useStore((s) => ownedStream(s.streams, owner))
  const restart = useCallback(() => setAttempt((n) => n + 1), [])

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

  if (!detail && detailsRead.error !== undefined) {
    return (
      <ErrorState
        title="Could not read the pod"
        detail={detailsRead.error}
        onRetry={detailsRead.retry}
      />
    )
  }

  const status = stream?.status ?? 'starting'
  const failedEmpty = status === 'error' && lines.length === 0

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
        {failedEmpty ? (
          <ErrorState
            title="Could not stream logs"
            detail={stream?.error ?? stream?.reason}
            onRetry={restart}
          />
        ) : (
          <div
            ref={scrollRef}
            onScroll={onScroll}
            data-testid="pod-logs-scroll"
            data-stream-status={status}
            className="h-full min-w-0 overflow-y-auto bg-background/40 p-2 font-mono text-xs leading-relaxed"
          >
            {status === 'starting' ? (
              <Loading rows={6} label={detail ? 'Starting log stream' : 'Loading pod'} />
            ) : lines.length === 0 ? (
              <p
                role="status"
                data-pane-state="logs-empty"
                className="font-sans text-xs text-muted-foreground"
              >
                {status === 'streaming' ? 'No log output yet.' : 'No log output.'}
              </p>
            ) : (
              <>
                {(stream?.lines.truncated ?? 0) > 0 && (
                  <p className="mb-1 text-xs text-muted-foreground">
                    … {stream?.lines.truncated} earlier line
                    {stream?.lines.truncated === 1 ? '' : 's'} truncated
                  </p>
                )}
                <pre className="min-w-0 break-words whitespace-pre-wrap text-foreground/90">
                  {lines.join('\n')}
                </pre>
              </>
            )}
          </div>
        )}
        {!follow && !failedEmpty && (
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
      {stream && (status === 'closed' || (status === 'error' && !failedEmpty)) && (
        <StreamEndBar stream={stream} onRestart={restart} />
      )}
    </div>
  )
}

function StreamEndBar({ stream, onRestart }: { stream: LogStreamState; onRestart: () => void }) {
  const failed = stream.status === 'error'
  const text = failed
    ? `Stream failed${stream.error ? `: ${stream.error}` : ''}`
    : stream.reason === 'stopped'
      ? 'Stream stopped'
      : `Stream ended${stream.reason && stream.reason !== 'ended' ? `: ${stream.reason}` : ''}`
  return (
    <div
      role={failed ? 'alert' : 'status'}
      data-pane-state={failed ? 'stream-error' : 'stream-ended'}
      className={cn(
        'flex shrink-0 items-center gap-2 border-t px-3 py-1.5 text-xs',
        failed
          ? 'border-status-err/40 bg-status-err/10 text-status-err'
          : 'border-status-warn/40 bg-status-warn/10 text-status-warn',
      )}
    >
      <WarningIcon aria-hidden="true" className="size-4 shrink-0" />
      <span className="min-w-0 flex-1 break-words">{text}</span>
      <Button type="button" variant="ghost" size="xs" onClick={onRestart}>
        <ArrowClockwiseIcon data-icon="inline-start" /> {failed ? 'Retry' : 'Restart'}
      </Button>
    </div>
  )
}
