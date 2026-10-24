/**
 * Beside the stack list on wide screens, before a stack is picked: how many stacks are in each
 * state, and what else is running on the host. Counts link to the list filtered by that state.
 * Admins also see the host's CPU, memory, disk and load (when the server reads its `/proc`), and
 * everyone sees CPU and memory per stack they may view, busiest first.
 */
import { useEffect, useMemo } from 'react'

import { Link } from '@/app/Link'
import { useStreams, watchStatsAll } from '@/features/tabs/streams'
import { MAX_STATS_TARGETS, type StackStatus } from '@/protocol/generated'

import { useCanSeeHost } from './host-store'
import { HostPanel } from './HostMetrics'
import { formatBytes, formatPercent, stackUsages, watchedStacks } from './metrics'
import { STATUS_LABEL, STATUS_ORDER, stackHref } from './model'
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

/** Watches the named stacks' stats while mounted; the server sends only those we may view. */
function useStackStats(names: string[]): void {
  const key = names.join('\n')
  useEffect(() => {
    if (key === '') return
    // Stack names are compose project names: never a newline.
    return watchStatsAll(key.split('\n').map((stack) => [`overview:${stack}`, { kind: 'stack', stack }] as const))
  }, [key])
}

function StackUsageTable() {
  const list = useStacks((s) => s.list)
  const samples = useStreams((s) => s.samples)
  const names = useMemo(() => watchedStacks(list, MAX_STATS_TARGETS), [list])
  useStackStats(names)
  const rows = useMemo(() => stackUsages(list, samples), [list, samples])
  if (rows.length === 0) return null
  return (
    <section data-section="stack-usage" aria-labelledby="usage-title" className="flex min-w-0 flex-col gap-2">
      <h2 id="usage-title" className="font-display text-xs font-semibold tracking-wider uppercase">
        Stacks by CPU
      </h2>
      <table className="w-full min-w-0 table-fixed border border-border text-xs">
        <thead className="text-muted-foreground">
          <tr className="border-b border-border">
            <th scope="col" className="px-2 py-1 text-left font-normal">
              Stack
            </th>
            <th scope="col" className="w-20 px-2 py-1 text-right font-normal">
              CPU
            </th>
            <th scope="col" className="w-24 px-2 py-1 text-right font-normal">
              Memory
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.name} data-stack-usage={r.name} className="border-b border-border last:border-b-0 hover:bg-muted">
              <th scope="row" className="truncate px-2 py-1 text-left font-medium">
                <Link to={stackHref(r.name)} className="text-foreground hover:text-primary focus-visible:text-primary">
                  {r.name}
                </Link>
              </th>
              <td data-field="cpu" className="px-2 py-1 text-right tabular-nums">
                {formatPercent(r.cpu)}
              </td>
              <td data-field="memory" className="px-2 py-1 text-right tabular-nums">
                {formatBytes(r.memory)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  )
}

export default function StacksOverview() {
  const showHost = useCanSeeHost()
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
      {showHost && <HostPanel />}
      <StackUsageTable />
      <p className="text-xs text-muted-foreground">Pick a stack from the list to see its services, files and logs.</p>
    </section>
  )
}
