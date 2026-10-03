/**
 * 048-docker-management US6: what a container is using, and what a project is using together.
 *
 * Every function here answers one question honestly: an unmeasured container is not an idle one.
 * The runtime writes `--` for a container it cannot measure, and the backend turns that into
 * `null` rather than `0` (FR-038) — so the arithmetic on this side has to keep the distinction
 * all the way to the screen. Summing `null` as zero would make a project of four containers where
 * two did not report look like a project that is half as busy as it is, which is the one reading
 * an operator cannot recover from on their own.
 *
 * A project's figure is therefore three numbers, not one: the total of what was measured, how many
 * members that was, and how many did not report. The panel says all three.
 */
import type { UsageSample } from '@/protocol/types'

/** How many samples of one container are kept. The backend's own cap (`stats::HISTORY_SAMPLES`). */
export const USAGE_HISTORY = 60

/** One metric, summed over the members that reported it. */
export interface Aggregate {
  /** The sum over reporting members. `null` when nobody reported, which is not the same as 0. */
  total: number | null
  /** How many members reported this metric. */
  measured: number
  /** How many did not — missing from the tick, or reported as unmeasurable. */
  unavailable: number
}

/** One project's members, added up without inventing the figures they did not give. */
export interface ProjectUsage {
  members: number
  cpu: Aggregate
  memory: Aggregate
}

function fold(values: (number | null | undefined)[]): Aggregate {
  let total = 0
  let measured = 0
  let unavailable = 0
  for (const value of values) {
    if (value === null || value === undefined || !Number.isFinite(value)) {
      unavailable += 1
      continue
    }
    total += value
    measured += 1
  }
  return { total: measured === 0 ? null : total, measured, unavailable }
}

/**
 * Add up one project's members from the latest tick.
 *
 * A member with no sample at all counts as unavailable rather than being left out of the
 * denominator: "3 of 4 reported" is a fact about the project, and dropping the fourth would
 * silently narrow the project to the part of it that answered.
 */
export function aggregate(
  containerIds: string[],
  latest: Record<string, UsageSample | undefined>,
): ProjectUsage {
  const samples = containerIds.map((id) => latest[id])
  return {
    members: containerIds.length,
    cpu: fold(samples.map((s) => s?.cpu_percent)),
    memory: fold(samples.map((s) => s?.memory_bytes)),
  }
}

/** The highest CPU figure in the kept history, or `null` if none of it was measured. */
export function peakCpu(history: UsageSample[] | undefined): number | null {
  let peak: number | null = null
  for (const sample of history ?? []) {
    const value = sample.cpu_percent
    if (value === null || !Number.isFinite(value)) continue
    if (peak === null || value > peak) peak = value
  }
  return peak
}

/** A CPU figure as a person reads it. `null` is said, not rounded to zero (FR-038). */
export function percentLabel(value: number | null): string {
  return value === null ? 'unavailable' : `${value.toFixed(1)}%`
}

/**
 * What a project's panel says about the members that did not report, or `null` when they all did.
 *
 * Phrased as a count of containers rather than of figures: the operator's next move is to go and
 * look at those containers, and a count of missing numbers would not tell them which.
 */
export function unavailableNote(usage: ProjectUsage): string | null {
  const missing = Math.max(usage.cpu.unavailable, usage.memory.unavailable)
  if (missing === 0) return null
  return `${String(missing)} of ${String(usage.members)} container${usage.members === 1 ? '' : 's'} did not report usage, so this total is of the ${String(usage.members - missing)} that did.`
}
