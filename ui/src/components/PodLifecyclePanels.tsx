/**
 * 008-pod-lifecycle-diagnostics: the Pod Inspector's "Lifecycle" Details section — init
 * containers distinct from app containers (US1), probe config + failure state (US2), per-
 * container restart history (US3), lifecycle hooks + failure correlation (US4), and the Pod's
 * own Events as an in-place timeline (US5). All data-testids are prefixed `pod-lifecycle-` to
 * avoid collision with other in-flight Pod Inspector work on the same page (010).
 *
 * `DetailSection.render` (ui/src/resources/types.ts) is a pure function with no hook access, so
 * this file's root component (`PodLifecyclePanels`) is the one place that reads the store and
 * `send`s `StartPodLogs`; it is plugged into `pods.tsx`'s `lifecycle` section as a
 * `{ type: 'list', items: [...] }` block item (`DetailBlock.items` is `ReactNode[]`). It sources
 * the Pod's Events by selecting the raw cluster-events list and filtering it in a `useMemo` —
 * mirroring `Inspector.tsx`'s own `EventsFor` — rather than filtering inside the `useStore`
 * selector itself, since a selector must return a stable reference, not a freshly filtered array
 * on every call (React's `useSyncExternalStore` otherwise loops).
 */
import { useMemo } from 'react'

import { StatusBadge } from '@/components/StatusBadge'
import { SECTION } from '@/protocol/commands'
import type { ContainerLifecycle, ContainerState, KubernetesEvent } from '@/protocol/types'
import { formatAge } from '@/resources/format'
import { hookState, probeState } from '@/resources/podLifecycle'
import { listKey, type ListRow, useStore } from '@/store'
import { cn } from '@/lib/utils'

const NO_ROWS: readonly ListRow[] = []

function containerStatusLabel(state: ContainerState): string {
  if (state.phase === 'terminated') return state.reason ?? 'Terminated'
  if (state.phase === 'running') return 'Running'
  return state.reason ?? 'Waiting'
}

function ContainerStateBadge({ state }: { state: ContainerState }) {
  return <StatusBadge status={containerStatusLabel(state)} />
}

function ViewLogsButton({
  namespace,
  name,
  container,
}: {
  namespace: string
  name: string
  container: string
}) {
  const requestPodLogs = useStore((s) => s.requestPodLogs)
  return (
    <button
      type="button"
      data-testid="pod-lifecycle-view-logs"
      onClick={() => {
        requestPodLogs(namespace, name, container)
      }}
      className="text-xs text-accent-neon hover:underline"
    >
      View logs
    </button>
  )
}

function InitContainersSection({
  namespace,
  name,
  containers,
}: {
  namespace: string
  name: string
  containers: ContainerLifecycle[]
}) {
  const initContainers = containers.filter((c) => c.is_init)
  return (
    <div data-testid="pod-lifecycle-init-containers" className="flex flex-col gap-2">
      <h4 className="font-display text-xs tracking-wider text-muted-foreground uppercase">
        Init Containers
      </h4>
      {initContainers.length === 0 ? (
        <p data-testid="pod-lifecycle-init-containers-empty" className="text-xs text-muted-foreground">
          No init containers.
        </p>
      ) : (
        <ol className="flex flex-col gap-1.5">
          {initContainers.map((c) => (
            <li
              key={c.name}
              data-testid={`pod-lifecycle-init-container-${c.name}`}
              className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-border/50 pb-1.5 text-sm"
            >
              <span className="font-mono font-medium">{c.name}</span>
              <ContainerStateBadge state={c.state} />
              {c.state.phase === 'terminated' && c.state.exit_code !== null && (
                <span className="font-mono text-xs text-muted-foreground">
                  exit {String(c.state.exit_code)}
                </span>
              )}
              <span className="ml-auto">
                <ViewLogsButton namespace={namespace} name={name} container={c.name} />
              </span>
            </li>
          ))}
        </ol>
      )}
    </div>
  )
}

function ProbesSection({
  containers,
  events,
}: {
  containers: ContainerLifecycle[]
  events: readonly KubernetesEvent[]
}) {
  return (
    <div data-testid="pod-lifecycle-probes" className="flex flex-col gap-2">
      <h4 className="font-display text-xs tracking-wider text-muted-foreground uppercase">Probes</h4>
      <ol className="flex flex-col gap-1.5">
        {containers.map((c) => {
          const entries = probeState(c, events)
          return (
            <li key={c.name} className="flex flex-col gap-1 border-b border-border/50 pb-1.5 text-sm">
              <span className="font-mono font-medium">{c.name}</span>
              {entries.length === 0 ? (
                <p data-testid="pod-lifecycle-probes-empty" className="text-xs text-muted-foreground">
                  No probes configured.
                </p>
              ) : (
                <ul className="flex flex-col gap-1 pl-2">
                  {entries.map(({ probe, failing }) => (
                    <li
                      key={probe.probe_type}
                      data-testid={failing ? 'pod-lifecycle-probe-failing' : undefined}
                      className={cn(
                        'flex flex-wrap items-center gap-x-2 font-mono text-xs',
                        failing ? 'text-status-err' : 'text-muted-foreground',
                      )}
                    >
                      <span className="capitalize">{probe.probe_type}</span>
                      <span>
                        {probe.mechanism} {probe.target}
                      </span>
                      <span>
                        period {String(probe.period_seconds)}s · fail after{' '}
                        {String(probe.failure_threshold)} · ok after{' '}
                        {String(probe.success_threshold)}
                      </span>
                      {failing && <span className="text-status-err">failing</span>}
                    </li>
                  ))}
                </ul>
              )}
            </li>
          )
        })}
      </ol>
    </div>
  )
}

function RestartHistorySection({ containers }: { containers: ContainerLifecycle[] }) {
  return (
    <div data-testid="pod-lifecycle-restarts" className="flex flex-col gap-2">
      <h4 className="font-display text-xs tracking-wider text-muted-foreground uppercase">
        Restart History
      </h4>
      <ol className="flex flex-col gap-1.5">
        {containers.map((c) => {
          // restartPolicy: Never edge case — a terminated container with `restart_count === 0`
          // never populates `last_state`, so `state` itself carries the termination info.
          const last =
            c.last_state ?? (c.restart_count === 0 && c.state.phase === 'terminated' ? c.state : null)
          return (
            <li key={c.name} className="flex flex-col gap-1 border-b border-border/50 pb-1.5 text-sm">
              <span className="flex flex-wrap items-center gap-x-2">
                <span className="font-mono font-medium">{c.name}</span>
                <span className="font-mono text-xs text-muted-foreground">
                  restarts: {String(c.restart_count)}
                </span>
              </span>
              {c.restart_count === 0 && !last ? (
                <p className="text-xs text-muted-foreground">No restarts.</p>
              ) : last ? (
                <p className="font-mono text-xs text-muted-foreground">
                  last: {last.reason ?? 'Terminated'}
                  {last.exit_code !== null && ` (exit ${String(last.exit_code)})`}
                  {last.finished_at && ` at ${last.finished_at}`}
                </p>
              ) : null}
            </li>
          )
        })}
      </ol>
    </div>
  )
}

function HooksSection({
  containers,
  events,
}: {
  containers: ContainerLifecycle[]
  events: readonly KubernetesEvent[]
}) {
  return (
    <div data-testid="pod-lifecycle-hooks" className="flex flex-col gap-2">
      <h4 className="font-display text-xs tracking-wider text-muted-foreground uppercase">
        Lifecycle Hooks
      </h4>
      <ol className="flex flex-col gap-1.5">
        {containers.map((c) => {
          const entries = hookState(c, events)
          return (
            <li key={c.name} className="flex flex-col gap-1 border-b border-border/50 pb-1.5 text-sm">
              <span className="font-mono font-medium">{c.name}</span>
              {entries.length === 0 ? (
                <p data-testid="pod-lifecycle-hooks-empty" className="text-xs text-muted-foreground">
                  No hooks configured.
                </p>
              ) : (
                <ul className="flex flex-col gap-1 pl-2">
                  {entries.map(({ hook, failed }) => (
                    <li
                      key={hook.hook_type}
                      data-testid={failed ? 'pod-lifecycle-hook-failed' : undefined}
                      className={cn(
                        'flex flex-wrap items-center gap-x-2 font-mono text-xs',
                        failed ? 'text-status-err' : 'text-muted-foreground',
                      )}
                    >
                      <span>{hook.hook_type}</span>
                      <span>
                        {hook.mechanism} {hook.target}
                      </span>
                      {failed && <span className="text-status-err">failed</span>}
                    </li>
                  ))}
                </ul>
              )}
            </li>
          )
        })}
      </ol>
    </div>
  )
}

function PodEventsTimeline({ events }: { events: readonly KubernetesEvent[] }) {
  const sorted = [...events].sort(
    (a, b) => Date.parse(a.last_timestamp) - Date.parse(b.last_timestamp),
  )
  return (
    <div data-testid="pod-lifecycle-events-timeline" className="flex flex-col gap-2">
      <h4 className="font-display text-xs tracking-wider text-muted-foreground uppercase">Events</h4>
      {sorted.length === 0 ? (
        <p data-testid="pod-lifecycle-events-timeline-empty" className="text-xs text-muted-foreground">
          No events.
        </p>
      ) : (
        <ol className="flex flex-col gap-1.5">
          {sorted.map((e) => {
            const warn = e.event_type === 'Warning'
            return (
              <li
                key={`${e.name}-${e.last_timestamp}`}
                className="flex gap-2 border-b border-border/50 pb-1.5 text-sm"
              >
                <span
                  aria-hidden="true"
                  className={cn('mt-0.5 shrink-0', warn ? 'text-status-warn' : 'text-muted-foreground')}
                >
                  {warn ? '▲' : '●'}
                </span>
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <p className="flex flex-wrap items-baseline gap-x-2">
                    <span className="font-medium">{e.reason}</span>
                    <span className="text-xs text-muted-foreground">{e.event_type}</span>
                    <span className="ml-auto font-mono text-xs text-muted-foreground">
                      {formatAge(e.last_timestamp)} ago
                    </span>
                  </p>
                  <p className="text-xs break-words text-muted-foreground">{e.message}</p>
                </div>
              </li>
            )
          })}
        </ol>
      )}
    </div>
  )
}

/**
 * The `lifecycle` `DetailSection`'s sole `list` item (`ui/src/resources/pods.tsx`). Sources the
 * Pod's Events once (filtered in a `useMemo`, matching `Inspector.tsx`'s `EventsFor`) and shares
 * them across the probe/hook failure correlation and the events timeline (FR-016 — no duplicate
 * fetch).
 */
export function PodLifecyclePanels({
  namespace,
  name,
  containers,
}: {
  namespace: string
  name: string
  containers: ContainerLifecycle[]
}) {
  const ctx = useStore((s) => s.session.activeContext)
  const list = useStore((s) => s.lists[listKey(ctx, SECTION.events, 'Cluster', null)])
  const events = useMemo(
    () =>
      (list?.rows ?? NO_ROWS).filter(
        (e): e is KubernetesEvent =>
          typeof e === 'object' &&
          'involved_object_kind' in e &&
          e.involved_object_kind === 'Pod' &&
          e.involved_object_name === name &&
          e.namespace === namespace,
      ),
    [list, namespace, name],
  )
  return (
    <div className="flex flex-col gap-4">
      <InitContainersSection namespace={namespace} name={name} containers={containers} />
      <ProbesSection containers={containers} events={events} />
      <RestartHistorySection containers={containers} />
      <HooksSection containers={containers} events={events} />
      <PodEventsTimeline events={events} />
    </div>
  )
}
