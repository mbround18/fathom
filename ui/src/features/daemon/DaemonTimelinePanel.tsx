/** 036 US1: "While you were away" — controller-level events from the in-cluster daemon. */
import { WarningIcon } from '@phosphor-icons/react'
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { DaemonTimelineEntry, DerivedSeverity, TruncationMarker } from '@/protocol/types'

import { Empty } from '../../components/shell/PaneStates'
import {
  daemonState,
  formatTime,
  INSTALL_COMMAND,
  STATE_HELP,
  STATE_LABEL,
  useActiveDaemon,
} from './state'

const PAGE = 200

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

export function DaemonTimelinePanel() {
  const { context, daemon } = useActiveDaemon()
  const [shown, setShown] = useState(PAGE)
  const state = daemonState(daemon)

  if (state !== 'available' && daemon.timeline.length === 0) {
    return (
      <div data-testid="daemon-timeline-empty" data-daemon-state={state}>
        <Empty
          title={`In-cluster daemon: ${STATE_LABEL[state]}`}
          detail={
            <>
              {STATE_HELP[state]}
              {state === 'off' || state === 'notInstalled' ? (
                <>
                  {' '}
                  Install (copy only): <code>{INSTALL_COMMAND}</code>
                </>
              ) : null}
            </>
          }
        />
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
      {daemon.truncated && <TruncationBanner marker={daemon.truncated} />}
      {state !== 'available' && (
        <p className="border-b border-border px-3 py-1 text-xs text-status-warn" role="status">
          Daemon {STATE_LABEL[state].toLowerCase()}: showing what was received earlier.
        </p>
      )}
      {daemon.timeline.length === 0 ? (
        <Empty title="Nothing happened while you were away" />
      ) : (
        <ul
          className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto"
          aria-label="While you were away"
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
