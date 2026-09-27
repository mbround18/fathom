/**
 * Standard pane states (FR-015, contracts/theme.md § Effects). Every pane renders one of these
 * instead of a blank area. Empty and error states carry the circuit-trace art.
 */
import {
  ArrowsClockwiseIcon,
  FileXIcon,
  GaugeIcon,
  LockSimpleIcon,
  PlugsIcon,
  WarningIcon,
  type Icon,
} from '@phosphor-icons/react'
import type { ReactNode } from 'react'

import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { CircuitTrace } from '@/theme/CircuitTrace'

export interface StateFrameProps {
  icon?: Icon
  title: string
  detail?: ReactNode
  action?: ReactNode
  trace?: boolean
  tone?: 'muted' | 'warn' | 'err'
  kind: string
}

export function StateFrame({
  icon: IconCmp,
  title,
  detail,
  action,
  trace,
  tone = 'muted',
  kind,
}: StateFrameProps) {
  return (
    <div
      role={tone === 'err' ? 'alert' : 'status'}
      aria-live={tone === 'err' ? 'assertive' : 'polite'}
      data-pane-state={kind}
      className="flex h-full min-h-40 flex-col items-center justify-center gap-3 p-6 text-center"
    >
      {trace && <CircuitTrace className="opacity-80" />}
      {IconCmp && (
        <IconCmp
          aria-hidden="true"
          className={cn(
            'size-6',
            tone === 'warn' && 'text-status-warn',
            tone === 'err' && 'text-status-err',
            tone === 'muted' && 'text-muted-foreground',
          )}
        />
      )}
      <p className="font-display text-sm font-semibold tracking-wider uppercase">{title}</p>
      {detail && <p className="max-w-prose text-xs text-muted-foreground">{detail}</p>}
      {action}
    </div>
  )
}

export function Loading({ rows = 8, label = 'Loading' }: { rows?: number; label?: string }) {
  return (
    <div
      role="status"
      aria-label={label}
      data-pane-state="loading"
      className="flex flex-col gap-1 p-2"
    >
      {Array.from({ length: rows }, (_, i) => (
        <Skeleton key={i} className="h-[calc(var(--row-height)-8px)] w-full" />
      ))}
    </div>
  )
}

export function Empty({ title = 'Nothing here', detail }: { title?: string; detail?: ReactNode }) {
  return <StateFrame kind="empty" title={title} detail={detail} trace />
}

export function ErrorState({
  title = 'Something failed',
  detail,
  onRetry,
}: {
  title?: string
  detail?: ReactNode
  onRetry?: () => void
}) {
  return (
    <StateFrame
      kind="error"
      icon={WarningIcon}
      tone="err"
      title={title}
      detail={detail}
      trace
      action={
        onRetry && (
          <Button variant="outline" size="sm" onClick={onRetry}>
            <ArrowsClockwiseIcon data-icon="inline-start" /> Retry
          </Button>
        )
      }
    />
  )
}

/** Never echoes backend/JS exception text: that lives in logs, not in front of the operator. */
export function Disconnected() {
  return (
    <StateFrame
      kind="disconnected"
      icon={PlugsIcon}
      tone="warn"
      title="Disconnected"
      detail="The backend stopped. Restart Fathom to reconnect; data shown may be stale."
    />
  )
}

export function NoLongerExists({ name }: { name?: string }) {
  return (
    <StateFrame
      kind="gone"
      icon={FileXIcon}
      title="No longer exists"
      detail={
        name ? `${name} was deleted from the cluster.` : 'This object was deleted from the cluster.'
      }
    />
  )
}

/**
 * 031: the identity's RBAC does not allow reading this section (the API server answered 403).
 * Not an error: nothing is broken and retrying will not help, so no Retry button.
 */
export function NotPermitted({ what, detail }: { what?: string; detail?: ReactNode }) {
  return (
    <StateFrame
      kind="not-permitted"
      icon={LockSimpleIcon}
      title="Not permitted"
      detail={
        detail ??
        `Your Kubernetes identity is not allowed to read ${what ?? 'this'} in the selected scope. Ask a cluster admin for access, or pick a namespace you can read.`
      }
    />
  )
}

/**
 * Also the integration panes' "scanning" / "not installed" / "nothing configured" state: pass a
 * `title` there so the heading does not claim metrics are the problem.
 */
export function MetricsUnavailable({
  detail,
  title = 'Metrics unavailable',
}: {
  detail?: ReactNode
  title?: string
}) {
  return (
    <StateFrame
      kind="metrics-unavailable"
      icon={GaugeIcon}
      title={title}
      detail={detail ?? 'metrics-server is not installed or not reachable in this cluster.'}
    />
  )
}
