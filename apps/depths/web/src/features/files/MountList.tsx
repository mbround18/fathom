/**
 * The Files tab's mount list (spec 053, T035): every mount of the stack, grouped by the service
 * that mounts it. A mount two services share is listed under each, with that service's container
 * path and read-only state. Each row names its source (volume name or host path), its type, and,
 * when it cannot be opened, why: not created yet, unresolved, missing, or not browsable (a tmpfs, a
 * single-file bind, a bind of a system path).
 *
 * `compact` is the narrow tree beside the listing on the widest layout (FR-023): one line per
 * mount, the current one marked.
 */
import { Link } from '@/app/Link'
import type { Mount, MountState } from '@/protocol/generated'

import { filesHref } from './model'

const STATE_TEXT: Record<Exclude<MountState, 'ready'>, string> = {
  not_created: 'Not created yet: Docker creates it on the first deploy.',
  unresolved: 'Its source could not be resolved from the compose file.',
  missing: 'Its source does not exist on the host.',
}

/** Whether a mount can be opened: its source exists and it is safe to browse. */
function canBrowse(m: Mount): boolean {
  return m.state === 'ready' && m.browsable.ok
}

interface Group {
  service: string
  rows: { mount: Mount; destination: string; readOnly: boolean }[]
}

function byService(mounts: readonly Mount[]): Group[] {
  const groups = new Map<string, Group>()
  for (const mount of mounts) {
    for (const s of mount.services) {
      let g = groups.get(s.service)
      if (!g) {
        g = { service: s.service, rows: [] }
        groups.set(s.service, g)
      }
      g.rows.push({ mount, destination: s.destination, readOnly: s.read_only })
    }
  }
  return [...groups.values()].sort((a, b) => a.service.localeCompare(b.service))
}

export interface MountListProps {
  stack: string
  mounts: readonly Mount[] | null
  /** The mount being browsed, marked in the compact tree. */
  current?: string | null
  compact?: boolean
}

export function MountList({ stack, mounts, current = null, compact = false }: MountListProps) {
  if (mounts === null) {
    return (
      <div data-pane="mounts" className="min-w-0">
        <p role="status" data-pane-state="loading" className="text-xs text-muted-foreground">
          Loading mounts…
        </p>
      </div>
    )
  }
  if (mounts.length === 0) {
    return (
      <div data-pane="mounts" className="min-w-0">
        <p role="status" data-pane-state="empty" className="text-xs text-muted-foreground">
          No mounts yet.
        </p>
      </div>
    )
  }
  const groups = byService(mounts)
  const notCreated = mounts.some((m) => m.state === 'not_created')
  return (
    <div data-pane="mounts" className="flex min-w-0 flex-col gap-2">
      {notCreated && !compact && (
        <p data-field="not-created-note" className="text-xs break-words text-muted-foreground">
          Named volumes appear after the first deploy: until then there is nothing to browse.
        </p>
      )}
      {groups.map((g) => (
        <section key={g.service} data-service={g.service} aria-labelledby={`mounts-${g.service}`} className="min-w-0 border border-border bg-card">
          <h3 id={`mounts-${g.service}`} className="truncate border-b border-border px-2 py-1 text-xs font-semibold">
            {g.service}
          </h3>
          <ul className="flex min-w-0 flex-col">
            {g.rows.map((r) => (
              <MountRow key={`${r.mount.id}:${r.destination}`} stack={stack} {...r} current={r.mount.id === current} compact={compact} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  )
}

function MountRow({
  stack,
  mount,
  destination,
  readOnly,
  current,
  compact,
}: {
  stack: string
  mount: Mount
  destination: string
  readOnly: boolean
  current: boolean
  compact: boolean
}) {
  const open = canBrowse(mount)
  const why = mount.state !== 'ready' ? STATE_TEXT[mount.state] : mount.browsable.ok ? null : `Not browsable: ${mount.browsable.reason?.message ?? 'refused'}.`
  return (
    <li
      data-mount-row={mount.id}
      data-state={mount.state}
      data-browsable={mount.browsable.ok ? undefined : 'false'}
      className={`flex min-w-0 flex-col gap-0.5 border-t border-border px-2 py-1.5 text-xs first:border-t-0 ${current ? 'bg-accent' : ''}`}
    >
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
        {open ? (
          <Link
            to={filesHref(stack, { mount: mount.id, path: '' })}
            aria-current={current ? 'page' : undefined}
            className="flex min-h-7 min-w-0 items-center font-mono font-medium break-all hover:text-primary"
          >
            {mount.source}
          </Link>
        ) : (
          <span className="min-w-0 font-mono font-medium break-all text-muted-foreground">{mount.source}</span>
        )}
        <span data-field="kind" className="shrink-0 border border-border px-1 text-[0.65rem] uppercase">
          {mount.kind}
        </span>
      </div>
      {!compact && (
        <p className="flex min-w-0 flex-wrap gap-x-2 text-muted-foreground">
          <span>
            at <span className="font-mono break-all text-foreground">{destination}</span>
          </span>
          <span data-field="access">{readOnly ? 'read-only' : 'read-write'}</span>
        </p>
      )}
      {why !== null && (
        <p data-field="why" className="min-w-0 break-words text-muted-foreground">
          {why}
        </p>
      )}
    </li>
  )
}
