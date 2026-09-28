/**
 * 036 US1: controller-level events from the in-cluster daemon. 043 US4 renamed it from "While
 * you were away" to "Cluster history" and made it open with a one-line explanation; when the
 * daemon is not available it explains why the panel is empty and how to turn it on.
 */
import { WarningIcon } from '@phosphor-icons/react'
import { useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { useStore } from '@/store'
import type { DaemonTimelineEntry, DerivedSeverity, TruncationMarker } from '@/protocol/types'

import { Empty } from '../../components/shell/PaneStates'
import {
  CLUSTER_HISTORY_SUBTITLE,
  DAEMON_SWITCH_LABEL,
  daemonState,
  formatTime,
  INSTALL_COMMAND,
  STATE_HELP,
  STATE_LABEL,
  useActiveDaemon,
} from './state'

const PAGE = 200
/** How much buffered history to ask the backend for when the pane opens or refreshes. */
export const TIMELINE_REQUEST_LIMIT = 1000

const SEV: Record<DerivedSeverity, { glyph: string; label: string; cls: string }> = {
  info: { glyph: '●', label: 'Info', cls: 'text-muted-foreground' },
  warning: { glyph: '▲', label: 'Warning', cls: 'text-status-warn' },
  error: { glyph: '✕', label: 'Error', cls: 'text-status-err' },
}

export function TruncationBanner({ marker }: { marker: TruncationMarker }) {
  const why =
    marker.reason === 'epochChanged'
      ? 'The daemon restarted, so earlier history is gone.'
      : 'Fathom was away longer than the daemon retains.'
  return (
    <div
      role="alert"
      data-testid="daemon-truncation-banner"
      className="flex items-start gap-2 border-b border-status-warn bg-muted px-3 py-2 text-xs"
    >
      <WarningIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-status-warn" />
      <span className="min-w-0 break-words">
        {why} History starts at {formatTime(marker.oldestTime)}
        {marker.dropped > 0 ? ` (${String(marker.dropped)} events dropped).` : '.'}
      </span>
    </div>
  )
}

function Entry({ e }: { e: DaemonTimelineEntry }) {
  const ev = e.event
  const sev = SEV[ev.severity]
  return (
    <li
      data-testid="daemon-timeline-entry"
      data-origin={ev.origin}
      data-severity={ev.severity}
      data-seq={e.seq}
      className="flex flex-col gap-0.5 border-b border-border px-3 py-2 text-sm"
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs">
        <span className={cn('flex items-center gap-1', sev.cls)}>
          <span aria-hidden="true">{sev.glyph}</span>
          {sev.label}
        </span>
        <span
          data-testid="daemon-origin-badge"
          className="border border-border px-1 text-[0.7rem] tracking-wide uppercase"
        >
          {ev.origin === 'derived' ? 'Derived' : 'Raw'}
        </span>
        <time className="text-muted-foreground" dateTime={e.time}>
          {formatTime(e.time)}
        </time>
        {ev.repeat > 1 && (
          <span className="text-muted-foreground" data-testid="daemon-repeat">
            x{ev.repeat}
          </span>
        )}
      </div>
      <p className="min-w-0 font-semibold break-words">
        {ev.kind} {ev.namespace ? `${ev.namespace}/` : ''}
        {ev.name}
        <span className="font-normal text-muted-foreground"> — </span>
        <span data-testid="daemon-reason">{ev.reason}</span>
      </p>
      {ev.message !== '' && (
        <p
          className="break-words whitespace-pre-wrap text-muted-foreground"
          data-testid="daemon-message"
        >
          {ev.message}
        </p>
      )}
    </li>
  )
}

function Subtitle() {
  return (
    <p
      data-testid="cluster-history-subtitle"
      className="shrink-0 border-b border-border px-3 py-1.5 text-xs text-muted-foreground"
    >
      {CLUSTER_HISTORY_SUBTITLE}
    </p>
  )
}

export function DaemonTimelinePanel() {
  const { context, daemon } = useActiveDaemon()
  const [shown, setShown] = useState(PAGE)
  const state = daemonState(daemon)
  const requestTimeline = useStore((s) => s.requestDaemonTimeline)
  const navigate = useStore((s) => s.navigate)

  // Opening the pane pulls the backend's buffered history (no server call on the backend side).
  useEffect(() => {
    if (context !== undefined && state === 'available') {
      requestTimeline(context, TIMELINE_REQUEST_LIMIT)
    }
  }, [context, state, requestTimeline])

  if (state !== 'available' && daemon.timeline.length === 0) {
    return (
      <div data-testid="daemon-timeline-empty" data-daemon-state={state}>
        <Subtitle />
        <Empty
          title={`No cluster history: daemon ${STATE_LABEL[state].toLowerCase()}`}
          detail={
            <span data-testid="cluster-history-unavailable">
              Cluster history comes from Fathom&apos;s optional, read-only in-cluster daemon.{' '}
              {STATE_HELP[state]}
              {state === 'off' ? (
                <>
                  {' '}
                  To use it, turn on “{DAEMON_SWITCH_LABEL}” in Settings › Cluster settings or in
                  the Daemon chip in the header.
                </>
              ) : null}
              {state === 'off' || state === 'notInstalled' ? (
                <>
                  {' '}
                  If the daemon is not installed, install it yourself (Fathom never does):{' '}
                  <code>{INSTALL_COMMAND}</code>
                </>
              ) : null}
            </span>
          }
        />
        {context !== undefined && (
          <div className="flex justify-center pb-3">
            <Button
              variant="outline"
              className="min-h-[var(--touch-target)]"
              data-testid="cluster-history-open-settings"
              onClick={() => navigate('integrations/cluster-settings')}
            >
              Open Cluster settings
            </Button>
          </div>
        )}
      </div>
    )
  }

  const newestFirst = [...daemon.timeline].reverse()
  const visible = newestFirst.slice(0, shown)

  return (
    <div
      className="flex h-full min-h-0 w-full min-w-0 flex-col"
      data-testid="daemon-timeline"
      data-context={context}
    >
      <Subtitle />
      {daemon.truncated && <TruncationBanner marker={daemon.truncated} />}
      {state !== 'available' && (
        <p className="border-b border-border px-3 py-1 text-xs text-status-warn" role="status">
          Daemon {STATE_LABEL[state].toLowerCase()}: showing what was received earlier.
        </p>
      )}
      {state === 'available' && context !== undefined && (
        <div className="flex justify-end border-b border-border px-3 py-1">
          <Button
            variant="outline"
            className="min-h-[var(--touch-target)]"
            data-testid="daemon-timeline-refresh"
            onClick={() => {
              requestTimeline(context, TIMELINE_REQUEST_LIMIT)
            }}
          >
            Refresh
          </Button>
        </div>
      )}
      {daemon.timeline.length === 0 ? (
        <Empty title="No changes recorded yet" />
      ) : (
        <ul
          className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto"
          aria-label="Cluster history"
        >
          {visible.map((e) => (
            <Entry key={e.seq} e={e} />
          ))}
          {newestFirst.length > shown && (
            <li className="p-2">
              <Button
                variant="outline"
                className="min-h-[var(--touch-target)] w-full"
                onClick={() => {
                  setShown((n) => n + PAGE)
                }}
              >
                Show older ({newestFirst.length - shown})
              </Button>
            </li>
          )}
        </ul>
      )}
    </div>
  )
}
