/**
 * Pure helpers behind the overview's metrics: formatting, per-stack sums of container samples,
 * and sparkline paths. No React, no socket, so they are unit tested directly.
 */
import type { HostSample, Stack, StackList, UsageSample } from '@/protocol/generated'

/** Samples kept per host figure for the sparklines (2 s apart: two minutes). */
export const HOST_HISTORY = 60

const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB']

/** `1536` → `1.5 KiB`; one decimal under 10, none above. */
export function formatBytes(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n) || n < 0) return '—'
  let v = n
  let i = 0
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024
    i++
  }
  return `${v >= 10 || i === 0 ? v.toFixed(0) : v.toFixed(1)} ${UNITS[i] ?? 'B'}`
}

/** `12.345` → `12.3%`; `null` → `—`. */
export function formatPercent(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '—'
  return `${n.toFixed(1)}%`
}

/** `90061` → `1d 1h`; under an hour `5m`; under a minute `42s`. */
export function formatUptime(secs: number | null | undefined): string {
  if (secs === null || secs === undefined || !Number.isFinite(secs) || secs < 0) return '—'
  const s = Math.floor(secs)
  const d = Math.floor(s / 86_400)
  const h = Math.floor((s % 86_400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (d > 0) return `${String(d)}d ${String(h)}h`
  if (h > 0) return `${String(h)}h ${String(m)}m`
  if (m > 0) return `${String(m)}m`
  return `${String(s)}s`
}

/** `part` of `whole` as 0–100, or `null` when there is no whole. */
export function share(part: number | null | undefined, whole: number | null | undefined): number | null {
  if (part === null || part === undefined || whole === null || whole === undefined || whole <= 0) return null
  return Math.max(0, Math.min(100, (part / whole) * 100))
}

/** The four figures the host panel draws as meters and sparklines, each 0–100 or `null`. */
export interface HostPoint {
  cpu: number | null
  memory: number | null
  disk: number | null
  /** One-minute load per core, as a percentage (100 = every core busy). */
  load: number | null
}

export function hostPoint(s: HostSample): HostPoint {
  const cpu = s.cpu_percent === null ? null : Math.max(0, Math.min(100, s.cpu_percent))
  const load = s.load && s.cpu_cores ? Math.max(0, Math.min(100, (s.load.one / s.cpu_cores) * 100)) : null
  return {
    cpu,
    memory: share(s.memory?.used_bytes, s.memory?.total_bytes),
    disk: share(s.disk?.used_bytes, s.disk?.total_bytes),
    load,
  }
}

/** One stack's containers, summed. `null` figures: no sample for any of its containers yet. */
export interface StackUsage {
  name: string
  cpu: number | null
  memory: number | null
  /** Containers with a sample, and in all. */
  measured: number
  containers: number
}

/** The sample for a container: by full id, or by a prefix either way (docker shortens ids). */
export function sampleFor(id: string, samples: Record<string, UsageSample>): UsageSample | undefined {
  return samples[id] ?? Object.values(samples).find((s) => id.startsWith(s.container_id) || s.container_id.startsWith(id))
}

export function stackUsage(stack: Stack, samples: Record<string, UsageSample>): StackUsage {
  const containers = stack.services.flatMap((v) => v.containers)
  let cpu: number | null = null
  let memory: number | null = null
  let measured = 0
  for (const c of containers) {
    const s = sampleFor(c.id, samples)
    if (!s) continue
    measured++
    if (s.cpu_percent !== null) cpu = (cpu ?? 0) + s.cpu_percent
    if (s.memory_bytes !== null) memory = (memory ?? 0) + s.memory_bytes
  }
  return { name: stack.name, cpu, memory, measured, containers: containers.length }
}

/**
 * Every stack with a container, busiest first (no figure last), then by name. Stacks with no
 * containers are left out: there is nothing to measure.
 */
export function stackUsages(list: StackList | null, samples: Record<string, UsageSample>): StackUsage[] {
  if (!list) return []
  return list.stacks
    .map((s) => stackUsage(s, samples))
    .filter((u) => u.containers > 0)
    .sort((a, b) => (b.cpu ?? -1) - (a.cpu ?? -1) || (b.memory ?? -1) - (a.memory ?? -1) || a.name.localeCompare(b.name))
}

/** The stacks the overview watches: those with a running container, by name, at most `max`. */
export function watchedStacks(list: StackList | null, max: number): string[] {
  if (!list) return []
  return list.stacks
    .filter((s) => s.services.some((v) => v.containers.some((c) => c.state === 'running')))
    .map((s) => s.name)
    .sort()
    .slice(0, max)
}

/**
 * An SVG path through `values` (0–100) across a `w`×`h` box, oldest at the left edge of the full
 * `HOST_HISTORY` window. Gaps (`null`) break the line. Empty when there are no points.
 */
export function sparkPath(values: readonly (number | null)[], w: number, h: number, slots = HOST_HISTORY): string {
  const step = slots > 1 ? w / (slots - 1) : 0
  const offset = Math.max(0, slots - values.length)
  let d = ''
  let pen = false
  values.forEach((v, i) => {
    if (v === null || !Number.isFinite(v)) {
      pen = false
      return
    }
    const x = (offset + i) * step
    const y = h - (Math.max(0, Math.min(100, v)) / 100) * h
    d += `${pen ? 'L' : 'M'}${x.toFixed(1)} ${y.toFixed(1)}`
    pen = true
  })
  return d
}
