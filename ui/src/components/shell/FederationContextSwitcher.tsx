/**
 * 012-incluster-daemon-federation P1: always-visible strip of every kubeconfig context with its
 * live connection status (FR-001, FR-002). Augments — does not replace — `ContextPicker`'s
 * combobox (contracts/context-status.md, research.md R4): switching still goes through the same
 * `selectContext` store action, so `us1-context-switch.spec.ts`'s combobox assertions keep
 * passing unmodified.
 */
import { useStore } from '@/store'
import type { ContextConnectionStatus } from '@/protocol/types'
import { cn } from '@/lib/utils'

/** `data-context-status` values (contracts/context-status.md). A context absent from
 * `session.contextStatuses` renders `not_connected` by convention (data-model.md). */
type StatusAttr = 'connected' | 'connecting' | 'error' | 'not_connected'

const STATUS_ATTR: Record<ContextConnectionStatus, StatusAttr> = {
  Connected: 'connected',
  Connecting: 'connecting',
  Error: 'error',
  NotConnected: 'not_connected',
}

const STATUS_LABEL: Record<StatusAttr, string> = {
  connected: 'Connected',
  connecting: 'Connecting…',
  error: 'Error',
  not_connected: 'Not connected',
}

const DOT_CLASS: Record<StatusAttr, string> = {
  connected: 'bg-emerald-400',
  connecting: 'animate-pulse bg-amber-400',
  error: 'bg-red-500',
  not_connected: 'bg-muted-foreground/40',
}

export function FederationContextSwitcher() {
  const contexts = useStore((s) => s.session.contexts)
  const active = useStore((s) => s.session.activeContext)
  const statuses = useStore((s) => s.session.contextStatuses)
  const selectContext = useStore((s) => s.selectContext)

  if (contexts.length === 0) return null

  return (
    <nav
      aria-label="Cluster contexts"
      data-testid="federation-context-switcher"
      className="flex w-full shrink-0 items-center gap-1.5 overflow-x-auto border-b border-border/60 bg-background/40 px-2 py-1.5"
    >
      {contexts.map((name) => {
        const entry = statuses[name]
        const statusAttr = entry ? STATUS_ATTR[entry.status] : 'not_connected'
        const isActive = name === active
        const title = entry?.error ?? STATUS_LABEL[statusAttr]
        return (
          <button
            key={name}
            type="button"
            data-testid="federation-context-item"
            data-context-name={name}
            data-context-status={statusAttr}
            aria-pressed={isActive}
            title={`${name}: ${title}`}
            onClick={() => selectContext(name)}
            className={cn(
              'flex min-h-[var(--touch-target)] shrink-0 items-center gap-1.5 rounded-md border px-2.5 font-mono text-xs whitespace-nowrap',
              isActive
                ? 'border-accent-neon/60 bg-accent-neon/10 text-foreground'
                : 'border-border/60 bg-background/30 text-muted-foreground hover:border-accent-neon/40',
            )}
          >
            <span
              aria-hidden="true"
              data-testid="federation-context-status-dot"
              className={cn('h-1.5 w-1.5 shrink-0 rounded-full', DOT_CLASS[statusAttr])}
            />
            {name}
            {statusAttr === 'error' && (
              <span data-testid="federation-context-error" className="sr-only">
                {entry?.error}
              </span>
            )}
          </button>
        )
      })}
    </nav>
  )
}
