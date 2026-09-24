import type { LimitKind } from '@/protocol/generated/LimitKind'

const NOUN: Record<LimitKind, string> = {
  sessions: 'concurrent sessions',
  watches: 'live watches',
  streams: 'open streams',
  tabs: 'browser tabs',
}

/** Inline, non-blocking notice; `role=status` so it is announced politely. `message` wins verbatim. */
export function LimitReached({
  kind,
  max,
  message,
  onDismiss,
}: {
  kind: LimitKind
  max: number
  message?: string
  onDismiss?: () => void
}) {
  return (
    <div
      role="status"
      className="flex items-start gap-3 border border-border bg-card px-3 py-2 text-sm"
    >
      <p className="flex-1">
        {message ??
          `Limit reached: you can have at most ${String(max)} ${NOUN[kind]}. Close one to open another.`}
      </p>
      {onDismiss && (
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss limit notice"
          className="h-6 px-2 text-xs hover:bg-muted focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
        >
          Dismiss
        </button>
      )}
    </div>
  )
}
