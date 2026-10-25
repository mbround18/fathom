/**
 * Session and network lists, shared by Your sessions and the admin panel on the Users page.
 * Location is display only (FR-019); browser strings are client-supplied and shown as short plain
 * text.
 */
import { Button } from '@companion/ui'

import { ANSWER_LABEL, day, REASON_LABEL, SOURCE_LABEL } from '@/features/network/model'
import { formatTime } from '@/features/stacks/model'
import type { NetworkAnswer, NetworkView, SessionView } from '@/protocol/generated'

import { shortBrowser, sortNetworks, sortSessions } from './model'

function Trust({ trusted, label }: { trusted: boolean; label: string }) {
  return (
    <span data-field="trust" data-trusted={trusted} className={trusted ? 'text-status-ok' : 'text-status-warn'}>
      {label}
    </span>
  )
}

export function SessionList({
  sessions,
  onEnd,
  busy = false,
  label,
}: {
  sessions: readonly SessionView[]
  onEnd?: (hint: string) => void
  busy?: boolean
  label: string
}) {
  if (sessions.length === 0) return <p className="text-xs text-muted-foreground">No active sessions.</p>
  return (
    <ul aria-label={label} className="flex min-w-0 flex-col gap-1">
      {sortSessions(sessions).map((s) => (
        <li
          key={s.id_hint}
          data-session={s.id_hint}
          data-current={s.current || undefined}
          className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 border border-border p-1.5 text-xs"
        >
          <span className="min-w-0 truncate font-semibold" data-field="browser" title={shortBrowser(s.browser, 200)}>
            {shortBrowser(s.browser)}
          </span>
          {s.current && (
            <span data-badge="current" className="border border-primary px-1 text-primary">
              This session
            </span>
          )}
          <span className="font-mono text-muted-foreground" data-field="address">
            {s.address}
          </span>
          <Trust trusted={s.trusted} label={s.trusted ? 'Trusted network' : 'Restricted network'} />
          <span className="text-muted-foreground" data-field="location">
            {s.location}
          </span>
          <span className="flex-1" />
          <span className="text-muted-foreground">
            Started {formatTime(s.created)} · last used {formatTime(s.last_used)}
          </span>
          {onEnd && !s.current && (
            <Button variant="outline" size="xs" disabled={busy} onClick={() => onEnd(s.id_hint)} aria-label={`End session ${s.id_hint}`}>
              End
            </Button>
          )}
        </li>
      ))}
    </ul>
  )
}

export function NetworkList({
  networks,
  label,
  onAnswer,
  onRemove,
  removeBlocked = null,
  busy = false,
}: {
  networks: readonly NetworkView[]
  label: string
  onAnswer?: (key: string, answer: NetworkAnswer) => void
  onRemove?: (n: NetworkView) => void
  /** Why removing is refused here (an untrusted network), or `null`. */
  removeBlocked?: string | null
  busy?: boolean
}) {
  if (networks.length === 0) return <p className="text-xs text-muted-foreground">No known networks.</p>
  return (
    <ul aria-label={label} className="flex min-w-0 flex-col gap-1">
      {sortNetworks(networks).map((n) => (
        <li
          key={n.key}
          data-network={n.key}
          data-current={n.current || undefined}
          data-reason={n.reason}
          className="flex min-w-0 flex-col gap-0.5 border border-border p-1.5 text-xs"
        >
          <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
            <span className="font-mono font-semibold">{n.key}</span>
            {n.current && (
              <span data-badge="current" className="border border-primary px-1 text-primary">
                This network
              </span>
            )}
            <Trust trusted={n.trusted} label={REASON_LABEL[n.reason]} />
            <span className="text-muted-foreground" data-field="location">
              {n.location}
            </span>
            <span className="flex-1" />
            <span className="text-muted-foreground">
              First {formatTime(n.first_used)} · last {formatTime(n.last_used)} · {String(n.days.length)} day
              {n.days.length === 1 ? '' : 's'}
            </span>
          </div>
          <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-muted-foreground">
            <span data-field="source">{SOURCE_LABEL[n.source]}</span>
            {n.label !== undefined && <span data-field="label">“{n.label}”</span>}
            {n.whitelisted_by !== undefined && (
              <span>
                by {n.whitelisted_by}
                {n.at !== undefined && ` on ${day(n.at)}`}
              </span>
            )}
            <span className="flex-1" />
            {onAnswer ? (
              <label className="inline-flex items-center gap-1">
                Answer
                <select
                  aria-label={`Answer for ${n.key}`}
                  value={n.answer}
                  disabled={busy}
                  onChange={(e) => onAnswer(n.key, e.target.value as NetworkAnswer)}
                  className="h-6 border border-input bg-background px-1 text-xs text-foreground"
                >
                  {n.answer === 'none' && <option value="none">{ANSWER_LABEL.none}</option>}
                  <option value="home">{ANSWER_LABEL.home}</option>
                  <option value="public">{ANSWER_LABEL.public}</option>
                </select>
              </label>
            ) : (
              <span data-field="answer">Answer: {ANSWER_LABEL[n.answer]}</span>
            )}
            {onRemove && (
              <Button
                variant="ghost"
                size="xs"
                disabled={busy || removeBlocked !== null}
                title={removeBlocked ?? undefined}
                data-blocked={removeBlocked === null ? undefined : 'untrusted_network'}
                onClick={() => onRemove(n)}
                aria-label={n.source === 'whitelist' ? `Remove whitelist ${n.key}` : `Forget ${n.key}`}
              >
                {n.source === 'whitelist' ? 'Remove whitelist' : 'Forget'}
              </Button>
            )}
          </div>
        </li>
      ))}
    </ul>
  )
}
