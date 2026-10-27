/**
 * Audit log filters (US5, T074): stack, actor, action and a time window. Choices apply at once;
 * the two text fields apply on Enter or "Apply", so a half-typed name never sends a query.
 */
import { FunnelIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { useState, type SyntheticEvent } from 'react'

import type { AuditAction, AuditFilter } from '@/protocol/generated'

import { ACTION_LABEL, ACTIONS, cleanFilter, isoToLocal, localToIso, type AuditFilterDraft } from './model'

const FIELD = 'h-7 min-w-0 border border-input bg-background px-1.5 text-xs outline-none focus:border-ring'

export function AuditFilters({
  filter,
  stacks,
  onChange,
}: {
  filter: AuditFilter
  /** Names to suggest; any name can be typed, since deleted stacks stay in the log. */
  stacks: readonly string[]
  onChange: (f: AuditFilter) => void
}) {
  const [stack, setStack] = useState(filter.stack ?? '')
  const [actor, setActor] = useState(filter.actor ?? '')

  const apply = (over: AuditFilterDraft = {}): void => {
    onChange(cleanFilter({ ...filter, stack: stack.trim(), actor: actor.trim(), ...over }))
  }
  const submit = (e: SyntheticEvent): void => {
    e.preventDefault()
    apply()
  }
  const clear = (): void => {
    setStack('')
    setActor('')
    onChange({})
  }

  return (
    <form
      onSubmit={submit}
      aria-label="Filter the audit log"
      className="flex min-w-0 flex-wrap items-center gap-1 border-b border-border px-2 py-1.5"
    >
      <FunnelIcon aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
      <input
        aria-label="Stack"
        placeholder="Stack"
        list="audit-stacks"
        value={stack}
        onChange={(e) => setStack(e.target.value)}
        className={`${FIELD} basis-28 flex-1`}
      />
      <datalist id="audit-stacks">
        {stacks.map((s) => (
          <option key={s} value={s} />
        ))}
      </datalist>
      <input
        aria-label="Actor id"
        placeholder="Actor id (github:…)"
        value={actor}
        onChange={(e) => setActor(e.target.value)}
        className={`${FIELD} basis-36 flex-1 font-mono`}
      />
      <select
        aria-label="Action"
        value={filter.action ?? ''}
        onChange={(e) => apply({ action: (e.target.value || undefined) as AuditAction | undefined })}
        className={`${FIELD} shrink-0 px-1`}
      >
        <option value="">Any action</option>
        {ACTIONS.map((a) => (
          <option key={a} value={a}>
            {ACTION_LABEL[a]}
          </option>
        ))}
      </select>
      <span className="inline-flex items-center gap-1">
        <span aria-hidden className="text-xs text-muted-foreground">
          From
        </span>
        <input
          type="datetime-local"
          aria-label="From"
          value={isoToLocal(filter.from)}
          onChange={(e) => apply({ from: localToIso(e.target.value) })}
          className={`${FIELD} shrink-0`}
        />
      </span>
      <span className="inline-flex items-center gap-1">
        <span aria-hidden className="text-xs text-muted-foreground">
          To
        </span>
        <input
          type="datetime-local"
          aria-label="To"
          value={isoToLocal(filter.to)}
          onChange={(e) => apply({ to: localToIso(e.target.value) })}
          className={`${FIELD} shrink-0`}
        />
      </span>
      <Button type="submit" variant="outline" size="sm">
        Apply
      </Button>
      <Button type="button" variant="ghost" size="sm" onClick={clear}>
        Clear
      </Button>
    </form>
  )
}
