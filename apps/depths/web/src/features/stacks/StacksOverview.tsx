/**
 * Beside the stack list on wide screens, before a stack is picked: how many stacks are in each
 * state, and what else is running on the host. Counts link to the list filtered by that state.
 */
import { useMemo } from 'react'

import type { StackStatus } from '@/protocol/generated'

import { STATUS_LABEL, STATUS_ORDER } from './model'
import { useStacks } from './store'

const TONE: Record<StackStatus, string> = {
  running: 'text-status-ok',
  partial: 'text-status-warn',
  stopped: 'text-muted-foreground',
  not_deployed: 'text-muted-foreground',
  invalid: 'text-status-err',
}

function Tile({ label, value, tone = '', onClick }: { label: string; value: number; tone?: string; onClick?: () => void }) {
  const body = (
    <>
      <span className={`font-display text-2xl font-semibold tabular-nums ${tone}`}>{value}</span>
      <span className="text-xs text-muted-foreground">{label}</span>
    </>
  )
  const cls = 'flex min-w-0 flex-col items-start gap-0.5 border border-border bg-card p-3 text-left'
  return onClick ? (
    <button type="button" onClick={onClick} className={`${cls} hover:border-primary focus-visible:border-ring focus-visible:outline-none`}>
      {body}
    </button>
  ) : (
    <div className={cls}>{body}</div>
  )
}

export default function StacksOverview() {
  const list = useStacks((s) => s.list)
  const setStatus = useStacks((s) => s.setStatus)
  const counts = useMemo(() => {
    const out = new Map<StackStatus, number>()
    for (const s of list?.stacks ?? []) out.set(s.status, (out.get(s.status) ?? 0) + 1)
    return out
  }, [list])

  if (!list) {
    return (
      <p role="status" data-pane-state="loading" className="p-3 text-xs text-muted-foreground">
        Loading stacks…
      </p>
    )
  }

  return (
    <section data-page="overview" aria-labelledby="overview-title" className="flex min-w-0 flex-col gap-3 p-3">
      <h1 id="overview-title" className="font-display text-sm font-semibold tracking-wider uppercase">
        Overview
      </h1>
      <div className="grid min-w-0 grid-cols-2 gap-2 xl:grid-cols-3 2xl:grid-cols-6">
        <Tile label="Stacks" value={list.stacks.length} onClick={() => setStatus('all')} />
        {STATUS_ORDER.map((s) => (
          <Tile key={s} label={STATUS_LABEL[s]} value={counts.get(s) ?? 0} tone={TONE[s]} onClick={() => setStatus(s)} />
        ))}
      </div>
      {(list.foreign.length > 0 || list.standalone.length > 0) && (
        <div className="grid min-w-0 grid-cols-2 gap-2 xl:grid-cols-3 2xl:grid-cols-6">
          <Tile label="Projects outside the stacks directory" value={list.foreign.length} />
          <Tile label="Standalone containers" value={list.standalone.length} />
        </div>
      )}
      <p className="text-xs text-muted-foreground">Pick a stack from the list to see its services, files and logs.</p>
    </section>
  )
}
