import { WarningIcon } from '@phosphor-icons/react'

import { cn } from '@/lib/utils'
import { useStore, type ConnectionStatus } from '@/store'

const STATUS: Record<ConnectionStatus, { label: string; dot: string }> = {
  connecting: { label: 'Connecting', dot: 'bg-status-warn animate-pulse' },
  connected: { label: 'Connected', dot: 'bg-status-ok' },
  reconnecting: { label: 'Reconnecting', dot: 'bg-status-warn animate-pulse' },
  disconnected: { label: 'Disconnected', dot: 'bg-status-err' },
}

/** Bottom status line: connection, active context, last notice, dropped-event warning (T049). */
export function StatusBar() {
  const status = useStore((s) => s.conn.status)
  const dropped = useStore((s) => s.conn.dropped)
  const notice = useStore((s) => s.conn.lastNotice)
  const context = useStore((s) => s.session.activeContext)
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
          className={cn(
            'hidden min-w-0 flex-1 truncate sm:block',
            notice.level === 'warn' && 'text-status-warn',
            notice.level === 'error' && 'text-status-err',
          )}
        >
          {notice.message}
        </span>
      )}
      {dropped > 0 && (
        <span
          className="ml-auto flex shrink-0 items-center gap-1 text-status-warn"
          data-testid="dropped-warning"
        >
          <WarningIcon aria-hidden="true" className="size-3.5" />
          {dropped} dropped
        </span>
      )}
    </footer>
  )
}
