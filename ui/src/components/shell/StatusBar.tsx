import { SpinnerIcon, WarningIcon } from '@phosphor-icons/react'
import { Popover } from 'radix-ui'

import { cn } from '@companion/theme'
import { useStore, type ConnectionStatus } from '@/store'

const STATUS: Record<ConnectionStatus, { label: string; dot: string }> = {
  starting: { label: 'starting', dot: 'bg-status-warn animate-pulse' },
  ready: { label: 'ready', dot: 'bg-status-ok' },
  'backend-gone': { label: 'backend gone', dot: 'bg-status-err' },
}

/** Bottom status line: connection, active context, last notice, dropped-event warning (T049). */
export function StatusBar() {
  const status = useStore((s) => s.conn.status)
  const dropped = useStore((s) => s.conn.dropped)
  const notice = useStore((s) => s.conn.lastNotice)
  const context = useStore((s) => s.session.activeContext)
  const pendingIds = useStore((s) => Object.keys(s.pending).join(','))
  const pendingCount = pendingIds === '' ? 0 : pendingIds.split(',').length
  const { label, dot } = STATUS[status]

  return (
    <footer
      role="status"
      aria-live="polite"
      data-testid="status-bar"
      className="neon-chrome flex h-7 shrink-0 items-center gap-3 overflow-hidden border-t border-border bg-card px-3 text-xs text-muted-foreground"
    >
      <span className="flex shrink-0 items-center gap-1.5" data-conn-status={status}>
        <span aria-hidden="true" className={cn('size-2', dot)} />
        <span className="text-foreground">{label}</span>
      </span>
      <span aria-hidden="true" className="text-border">
        |
      </span>
      <span className="min-w-0 shrink truncate">
        ctx <span className="text-accent-neon">{context ?? 'none'}</span>
      </span>
      {notice && (
        <span
          data-testid="status-notice"
          data-level={notice.level}
          className={cn(
            'flex min-w-0 flex-1 items-center gap-2',
            notice.level === 'warn' && 'text-status-warn',
            notice.level === 'error' && 'text-status-err',
          )}
        >
          <span className="min-w-0 truncate" title={notice.message}>
            {notice.message}
          </span>
          <Popover.Root>
            <Popover.Trigger asChild>
              <button
                type="button"
                data-testid="status-notice-details"
                className="min-h-6 shrink-0 px-1 text-accent-neon underline underline-offset-2 outline-none focus-visible:ring-1 focus-visible:ring-ring"
              >
                Details
              </button>
            </Popover.Trigger>
            <Popover.Portal>
              <Popover.Content
                side="top"
                align="start"
                sideOffset={6}
                data-testid="status-notice-popover"
                className="z-50 max-h-64 w-96 max-w-[calc(100vw-1.5rem)] overflow-auto border border-border bg-popover p-3 text-xs break-words whitespace-pre-wrap text-popover-foreground shadow-lg"
              >
                {notice.message}
              </Popover.Content>
            </Popover.Portal>
          </Popover.Root>
        </span>
      )}
      {pendingCount > 0 && (
        <span
          className="ml-auto flex shrink-0 items-center gap-1 text-accent-neon"
          data-testid="pending-actions"
          data-request-ids={pendingIds}
        >
          <SpinnerIcon aria-hidden="true" className="size-3.5 animate-spin" />
          {pendingCount} pending
        </span>
      )}
      {dropped > 0 && (
        <span
          className={cn(
            'flex shrink-0 items-center gap-1 text-status-warn',
            pendingCount === 0 && 'ml-auto',
          )}
          data-testid="dropped-warning"
        >
          <WarningIcon aria-hidden="true" className="size-3.5" />
          {dropped} dropped
        </span>
      )}
    </footer>
  )
}
