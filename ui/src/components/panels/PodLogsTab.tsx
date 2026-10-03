/**
 * Logs tab (Phase 3, US1): live-tails a pod's log output.
 *
 * Every mounted logs view (inspector Logs tab, dock Logs tool, tools sheet) owns its own backend
 * stream under a client-chosen id (`startPodLogs`/`stopPodLogs`, feedback #2), so views never
 * stop each other's streams. The stream is found again by this view's `owner` tag. States:
 * starting (skeleton), streaming (lines, or "No log output yet" for a quiet container), closed,
 * and error (inline message with Retry). A failed pod-details read is an inline error too.
 */
import { useCallback, useEffect, useId, useState } from 'react'

import type { PodDetails } from '@/protocol/types'
import type { ObjectRef } from '@/resources'
import { detailKey, useStore, type LogStreamState, type StoreState } from '@/store'
import { waitForCommandResult } from '@/store/actions'

import { LogScroller } from './LogScroller'
import { Picker } from '../Picker'
import { ErrorState } from '../shell/PaneStates'

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

  if (!detail && detailsRead.error !== undefined) {
    return (
      <ErrorState
        title="Could not read the pod"
        detail={detailsRead.error}
        onRetry={detailsRead.retry}
      />
    )
  }

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
      <LogScroller
        lines={lines}
        truncated={stream?.lines.truncated ?? 0}
        rev={stream?.rev ?? 0}
        status={stream?.status ?? 'starting'}
        error={stream?.error}
        reason={stream?.reason}
        startingLabel={detail ? 'Starting log stream' : 'Loading pod'}
        errorTitle="Could not stream logs"
        onRestart={restart}
      />
    </div>
  )
}
