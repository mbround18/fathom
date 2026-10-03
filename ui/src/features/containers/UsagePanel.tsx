/**
 * 048-docker-management US6 (T071): what the containers on screen are using, while they are.
 *
 * Sampling starts when this panel mounts and stops when it unmounts, which is the whole of FR-045:
 * navigating away, closing the scene or selecting something else unmounts it, and none of those
 * paths has to remember to stop anything. The ids are explicit — the panel measures the containers
 * it is showing and nothing else, because this computer's daemon is shared with unrelated work and
 * an unscoped reading would sample all of it.
 *
 * Nothing here turns an absent figure into a number. A container the runtime could not measure
 * reads "unavailable", and a project's total says how many of its members are behind it (FR-038):
 * a sum that silently counted the unmeasured as idle would be a smaller, wronger number with
 * nothing on screen to give it away.
 */
import { useEffect } from 'react'

import type { Container } from '@/protocol/types'
import { useStore } from '@/store'

import { sizeLabel } from './labels'
import { aggregate, peakCpu, percentLabel, unavailableNote, USAGE_HISTORY } from './usage'

export function UsagePanel({ containers }: { containers: Container[] }) {
  const start = useStore((s) => s.startContainerUsage)
  const stop = useStore((s) => s.stopContainerUsage)
  const usage = useStore((s) => s.containerRuntime.usage)

  // Keyed on the ids themselves rather than the array, so a re-render that rebuilds the list
  // without changing what is on screen does not restart the sampling.
  const ids = containers.map((c) => c.id)
  const key = ids.join(',')

  useEffect(() => {
    start(key === '' ? [] : key.split(','))
    return () => {
      stop()
    }
  }, [key, start, stop])

  const project = aggregate(ids, usage.latest)
  const missing = unavailableNote(project)

  return (
    <section
      data-testid="container-usage"
      data-ticks={String(usage.ticks)}
      className="flex shrink-0 flex-col gap-1 border-t border-border px-3 py-2"
    >
      <div className="flex min-w-0 items-baseline gap-2">
        <h3 className="text-xs font-medium">Usage</h3>
        <p className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
          {usage.ticks === 0
            ? 'Waiting for the first reading…'
            : 'Read while this panel is on screen, and not otherwise.'}
        </p>
      </div>

      <ul className="flex flex-col">
        {containers.map((container) => {
          const sample = usage.latest[container.id]
          const peak = peakCpu(usage.history[container.id])
          return (
            <li
              key={container.id}
              data-testid={`container-usage-row-${container.id}`}
              className="flex min-w-0 items-baseline gap-2 py-0.5 text-xs"
            >
              <span className="min-w-0 flex-1 truncate font-mono">{container.name}</span>
              <span data-testid={`container-usage-cpu-${container.id}`} className="tabular-nums">
                CPU {percentLabel(sample?.cpu_percent ?? null)}
              </span>
              <span data-testid={`container-usage-mem-${container.id}`} className="tabular-nums">
                {/* The same word for both metrics: a container the runtime cannot measure is
                    usually unmeasurable in both, and "0 B" would read as "using nothing". */}
                Memory {sizeLabel(sample?.memory_bytes ?? null) ?? 'unavailable'}
              </span>
              <span className="w-24 text-right text-muted-foreground tabular-nums">
                peak {percentLabel(peak)}
              </span>
            </li>
          )
        })}
      </ul>

      {containers.length > 1 && (
        /* The project as one unit, which is the reading grouping exists for — and it is stated as
           a total *of the members that reported*, never of all of them. */
        <p
          data-testid="container-usage-total"
          className="border-t border-border/60 pt-1 text-xs tabular-nums"
        >
          {project.members} containers · CPU {percentLabel(project.cpu.total)} · Memory{' '}
          {sizeLabel(project.memory.total) ?? 'unavailable'}
        </p>
      )}

      {missing !== null && (
        <p
          role="status"
          data-testid="container-usage-unavailable"
          className="text-[11px] text-status-warn"
        >
          {missing}
        </p>
      )}

      {usage.atCap && (
        /* FR-038: the bound is admitted rather than implied. An operator reading a peak needs to
           know how far back the window it was taken over actually goes. */
        <p
          role="status"
          data-testid="container-usage-at-cap"
          className="text-[11px] text-muted-foreground"
        >
          Only the last {USAGE_HISTORY} readings are kept; older ones are being dropped.
        </p>
      )}
    </section>
  )
}
