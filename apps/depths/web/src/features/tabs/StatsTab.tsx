/**
 * CPU and memory for a container or every container of a stack. Watches only while on screen: the
 * tab host mounts it for the visible tab only, and the streams store drops the watch while the
 * page is hidden.
 */
import { useEffect, useMemo } from 'react'

import { useStacks } from '@/features/stacks/store'
import type { Container, StatsTarget, UsageSample } from '@/protocol/generated'

import type { TabSpec } from './layout'
import { useStreams, watchStats } from './streams'

function containersOf(target: StatsTarget, list: ReturnType<typeof useStacks.getState>['list']): Container[] {
  if (!list) return []
  if (target.kind === 'stack') return list.stacks.find((s) => s.name === target.stack)?.services.flatMap((v) => v.containers) ?? []
  const all = [
    ...list.stacks.flatMap((s) => s.services.flatMap((v) => v.containers)),
    ...list.foreign.flatMap((f) => f.containers),
    ...list.standalone,
  ]
  return all.filter((c) => c.name === target.container || c.id === target.container)
}

function bytes(n: number): string {
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let v = n
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v >= 10 || i === 0 ? v.toFixed(0) : v.toFixed(1)} ${units[i] ?? 'B'}`
}

function Meter({ label, value }: { label: string; value: number | null }) {
  const pct = value === null ? 0 : Math.max(0, Math.min(100, value))
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value === null ? undefined : Math.round(pct)}
      className="h-1 w-full bg-muted"
    >
      <div className="h-full bg-primary" style={{ width: `${String(pct)}%` }} />
    </div>
  )
}

function Row({ name, sample }: { name: string; sample: UsageSample | undefined }) {
  const cpu = sample?.cpu_percent ?? null
  const mem = sample?.memory_bytes ?? null
  const limit = sample?.memory_limit_bytes ?? null
  const memPct = mem !== null && limit !== null && limit > 0 ? (mem / limit) * 100 : null
  return (
    <li data-stats-row={name} className="flex min-w-0 flex-col gap-1 border-t border-border px-2 py-1.5 first:border-t-0">
      <span className="truncate text-xs font-medium">{name}</span>
      {sample === undefined ? (
        <span className="text-xs text-muted-foreground">Waiting for a sample…</span>
      ) : (
        <div className="grid grid-cols-[3.5rem_minmax(0,1fr)] items-center gap-x-2 gap-y-1 text-xs">
          <span className="text-muted-foreground">CPU</span>
          <div className="flex min-w-0 items-center gap-2">
            <span data-field="cpu" className="w-14 shrink-0 tabular-nums">
              {cpu === null ? 'n/a' : `${cpu.toFixed(1)}%`}
            </span>
            <Meter label={`CPU ${name}`} value={cpu} />
          </div>
          <span className="text-muted-foreground">Memory</span>
          <div className="flex min-w-0 items-center gap-2">
            <span data-field="memory" className="shrink-0 tabular-nums">
              {mem === null ? 'n/a' : bytes(mem)}
              {limit !== null && ` / ${bytes(limit)}`}
            </span>
            <Meter label={`Memory ${name}`} value={memPct} />
          </div>
        </div>
      )}
    </li>
  )
}

export function StatsTab({ tab }: { tab: Extract<TabSpec, { kind: 'stats' }> }) {
  const list = useStacks((s) => s.list)
  const load = useStacks((s) => s.load)
  const samples = useStreams((s) => s.samples)
  const target = tab.target
  const key = JSON.stringify(target)

  useEffect(() => load(), [load])
  // Re-watch only when the target itself changes, not on every render's fresh object.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => watchStats(tab.id, target), [tab.id, key])

  const containers = useMemo(() => containersOf(target, list), [target, list])
  const sampleFor = (c: Container): UsageSample | undefined =>
    samples[c.id] ?? Object.values(samples).find((s) => c.id.startsWith(s.container_id) || s.container_id.startsWith(c.id))

  return (
    <div data-tab-body="stats" className="h-full min-h-0 overflow-auto">
      {containers.length === 0 ? (
        <p role="status" data-pane-state="empty" className="p-2 text-xs text-muted-foreground">
          {list ? 'No containers to measure.' : 'Loading…'}
        </p>
      ) : (
        <ul>
          {containers.map((c) => (
            <Row key={c.id} name={c.name} sample={sampleFor(c)} />
          ))}
        </ul>
      )}
    </div>
  )
}
