/**
 * Rows for what runs on the host outside the stacks directory (FR-008): foreign compose projects
 * and standalone containers. Admins only (the server sends both empty to a `user`), and read-only
 * for everyone: no row here links to an action.
 */
import { memo } from 'react'

import type { Container, ForeignProject } from '@/protocol/generated'

import { stateLabel } from './model'

const RUNNING = 'running'

function runningCount(containers: readonly Container[]): number {
  return containers.filter((c) => c.state === RUNNING).length
}

export const ForeignRow = memo(function ForeignRow({ project }: { project: ForeignProject }) {
  return (
    <article
      data-foreign={project.name}
      aria-label={`${project.name} (not in the stacks directory)`}
      className="flex min-w-0 flex-col gap-0.5 border-b border-border px-2 py-1.5"
    >
      <div className="flex min-w-0 items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-sm">{project.name}</span>
        <span className="shrink-0 border border-border px-1.5 text-xs text-muted-foreground">Read-only</span>
      </div>
      <div className="flex min-w-0 flex-wrap gap-x-3 text-xs text-muted-foreground">
        <span>
          {runningCount(project.containers)}/{project.containers.length} containers running
        </span>
        {project.working_dir !== null && <span className="min-w-0 break-all">{project.working_dir}</span>}
      </div>
      {project.elsewhere_of !== null && (
        <p className="text-xs text-status-warn">Started by hand from stack {project.elsewhere_of}&apos;s directory</p>
      )}
    </article>
  )
})

export const StandaloneRow = memo(function StandaloneRow({ container }: { container: Container }) {
  return (
    <article
      data-standalone={container.name}
      aria-label={`${container.name} (standalone container)`}
      className="flex min-w-0 flex-col gap-0.5 border-b border-border px-2 py-1.5"
    >
      <div className="flex min-w-0 items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-sm">{container.name}</span>
        <span className="shrink-0 border border-border px-1.5 text-xs text-muted-foreground">Read-only</span>
      </div>
      <div className="flex min-w-0 flex-wrap gap-x-3 text-xs text-muted-foreground">
        <span>{stateLabel(container.state)}</span>
        <span className="min-w-0 break-all">{container.image}</span>
        <span>{container.status}</span>
      </div>
    </article>
  )
})

export function GroupHeading({ id, label, count }: { id: string; label: string; count: number }) {
  return (
    <h2
      id={id}
      className="flex items-center gap-2 border-b border-border bg-background px-2 pt-3 pb-1 font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase"
    >
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <span className="tabular-nums">{count}</span>
    </h2>
  )
}
