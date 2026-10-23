/**
 * 048 T069: the arithmetic of US6, which is mostly about refusing to do arithmetic.
 *
 * The runtime hands its figures over as terminal strings and writes `--` for a container it cannot
 * measure. The backend parses those (`container_runtime::stats`), and what reaches here is a
 * number or `null`. Every assertion below is the same claim from a different angle: `null` never
 * becomes `0`, in a label, in a total, or in a peak (FR-038). A project of four containers where
 * two did not report must not read as a project that is half as busy as it is.
 */
import { describe, expect, it } from 'vitest'

import { aggregate, peakCpu, percentLabel, unavailableNote } from '@/features/containers/usage'
import { initialData, useStore } from '@/store'
import { ingest } from '@/store/ingest'

import type { UsageSample } from '@/protocol/types'

import { usageSampled } from '../fixtures/containers'

const sample = (
  container_id: string,
  cpu_percent: number | null,
  memory_bytes: number | null,
): UsageSample => ({
  container_id,
  at: '2026-10-02T12:00:00Z',
  cpu_percent,
  memory_bytes,
  memory_limit_bytes: 2_000_000_000,
})

const byId = (samples: UsageSample[]) => Object.fromEntries(samples.map((s) => [s.container_id, s]))

describe('a figure the runtime did not give', () => {
  it('is said, not rounded to zero', () => {
    expect(percentLabel(null)).toBe('unavailable')
    expect(percentLabel(0)).toBe('0.0%')
    expect(percentLabel(12.34)).toBe('12.3%')
  })

  it('leaves a peak unavailable rather than making it the floor', () => {
    expect(peakCpu(undefined)).toBeNull()
    expect(peakCpu([sample('a', null, null)])).toBeNull()
    expect(peakCpu([sample('a', 3, 1), sample('a', null, 1), sample('a', 9, 1)])).toBe(9)
  })
})

describe('a project added up', () => {
  it('totals the members that reported and counts the ones that did not', () => {
    const usage = aggregate(
      ['a', 'b', 'c', 'd'],
      byId([sample('a', 10, 1_000), sample('b', 5, 2_000), sample('c', null, null)]),
    )
    expect(usage.members).toBe(4)
    // 'c' reported nothing and 'd' was not in the tick at all: both are unavailable, not idle.
    expect(usage.cpu).toEqual({ total: 15, measured: 2, unavailable: 2 })
    expect(usage.memory).toEqual({ total: 3_000, measured: 2, unavailable: 2 })
  })

  it('says how many members the total is actually of', () => {
    const usage = aggregate(['a', 'b', 'c', 'd'], byId([sample('a', 10, 1_000)]))
    expect(unavailableNote(usage)).toBe(
      '3 of 4 containers did not report usage, so this total is of the 1 that did.',
    )
  })

  it('says nothing when every member reported', () => {
    const usage = aggregate(['a'], byId([sample('a', 1, 2)]))
    expect(unavailableNote(usage)).toBeNull()
  })

  it('has no total at all when nobody reported, which is not a total of zero', () => {
    const usage = aggregate(['a', 'b'], {})
    expect(usage.cpu.total).toBeNull()
    expect(usage.memory.total).toBeNull()
    expect(usage.cpu.measured).toBe(0)
  })

  it('counts a figure that is not a number as unavailable', () => {
    // The backend parses `is_finite` before it emits, but the slice is the last place that can
    // keep a NaN off the screen as a figure, and `NaN%` is worse than "unavailable".
    const usage = aggregate(['a'], byId([sample('a', Number.NaN, 5)]))
    expect(usage.cpu).toEqual({ total: null, measured: 0, unavailable: 1 })
    expect(usage.memory.total).toBe(5)
  })
})

describe('the ticks, as the slice keeps them', () => {
  it('bounds the history and keeps the cap the backend reported visible', () => {
    useStore.setState(initialData())
    for (let i = 0; i < 70; i++) {
      ingest(usageSampled([sample('a', i, 1_000 + i)], i >= 60))
    }
    const usage = useStore.getState().containerRuntime.usage
    expect(usage.ticks).toBe(70)
    expect(usage.history.a).toHaveLength(60)
    // The oldest kept sample, not the oldest sampled: the window moved and says so.
    expect(usage.history.a?.[0]?.cpu_percent).toBe(10)
    expect(usage.latest.a?.cpu_percent).toBe(69)
    expect(usage.atCap).toBe(true)
  })

  it('does not un-say a bound a later tick happens not to mention', () => {
    useStore.setState(initialData())
    ingest(usageSampled([sample('a', 1, 1)], true))
    ingest(usageSampled([sample('a', 2, 2)], false))
    expect(useStore.getState().containerRuntime.usage.atCap).toBe(true)
  })
})
