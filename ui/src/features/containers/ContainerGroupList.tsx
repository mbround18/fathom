/**
 * 048-docker-management US1: what this computer is running, grouped by compose project.
 *
 * One flat virtualized list of headings and rows rather than a list of lists, because the thing an
 * operator does here is scroll past projects they are not looking for. Nested scroll containers
 * make that a hunt for the right one, and 500 containers in one project (the render-budget
 * scenario) would still be 500 live rows inside it.
 *
 * Each heading carries the project's name, its member count and how many of those are running,
 * because "shop" alone does not answer the question the scene exists for. The adoption badge sits
 * in the heading for the same reason: whether Fathom may change this project is a property of the
 * project, not of the row the operator happens to click (FR-012).
 */
import { useVirtualizer } from '@tanstack/react-virtual'
import { useMemo, useRef } from 'react'

import { cn } from '@/lib/utils'
import type { Container, Project } from '@/protocol/types'

import { ClusterLinkBadge } from './ClusterLinkBadge'
import { adoptionLabel, definitionLabel, groupKey, groupLabel, type Selection } from './labels'

/** Headings are taller than rows: two lines of text at phone width, one above the counts. */
const GROUP_HEIGHT = 64
const ROW_HEIGHT = 40

type Item =
  | { kind: 'group'; project: Project }
  | { kind: 'container'; container: Container; project: Project }

export function ContainerGroupList({
  projects,
  selected,
  onSelect,
}: {
  projects: Project[]
  /** Whatever the actions panel is open for, if anything. */
  selected: Selection | null
  onSelect: (selection: Selection) => void
}) {
  const items = useMemo<Item[]>(
    () =>
      projects.flatMap((project) => [
        { kind: 'group' as const, project },
        ...project.containers.map((container) => ({
          kind: 'container' as const,
          container,
          project,
        })),
      ]),
    [projects],
  )

  const scrollRef = useRef<HTMLDivElement>(null)
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => (items[i]?.kind === 'group' ? GROUP_HEIGHT : ROW_HEIGHT),
    overscan: 8,
  })

  return (
    <div
      ref={scrollRef}
      data-testid="container-group-list"
      className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto"
    >
      <div
        role="list"
        aria-label="Containers by project"
        style={{ height: virtualizer.getTotalSize(), position: 'relative' }}
      >
        {virtualizer.getVirtualItems().map((v) => {
          const item = items[v.index]
          if (!item) return null
          return (
            <div
              key={v.key}
              role="listitem"
              className="absolute inset-x-0"
              style={{ top: v.start, height: v.size }}
            >
              {item.kind === 'group' ? (
                <GroupHeading
                  project={item.project}
                  selected={selected?.kind === 'project' && selected.key === groupKey(item.project)}
                  onSelect={onSelect}
                />
              ) : (
                <ContainerRow
                  container={item.container}
                  selected={selected?.kind === 'container' && selected.id === item.container.id}
                  onSelect={onSelect}
                />
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}

function GroupHeading({
  project,
  selected,
  onSelect,
}: {
  project: Project
  selected: boolean
  onSelect: (selection: Selection) => void
}) {
  const definition = definitionLabel(project)
  const adopted = project.adoption !== 'not_adopted'
  const key = groupKey(project)
  return (
    // A button, because selecting a project is what opens its verbs: a heading that only looks
    // clickable is not reachable by keyboard, and this is the only route to a mutation.
    <button
      type="button"
      aria-pressed={selected}
      onClick={() => {
        onSelect({ kind: 'project', key })
      }}
      data-testid={`containers-group-${key}`}
      data-adoption={adopted ? 'adopted' : 'read-only'}
      data-selected={selected ? 'true' : 'false'}
      className={cn(
        'flex h-full w-full min-w-0 flex-col justify-center gap-0.5 border-b border-border px-3 text-left',
        selected ? 'bg-muted' : 'bg-muted/40',
      )}
    >
      <div className="flex min-w-0 items-baseline gap-2">
        <span className="truncate font-display text-xs font-semibold tracking-wider uppercase">
          {groupLabel(project)}
        </span>
        <span
          data-testid={`containers-group-counts-${groupKey(project)}`}
          className="shrink-0 text-xs text-muted-foreground"
        >
          {project.running} running of {project.members}
        </span>
        <span
          data-testid={`containers-group-adoption-${groupKey(project)}`}
          className={cn(
            'ml-auto shrink-0 border px-1.5 py-0.5 text-[10px] tracking-wider uppercase',
            adopted ? 'border-status-ok/60 text-status-ok' : 'border-border text-muted-foreground',
          )}
        >
          {adoptionLabel(project)}
        </span>
      </div>
      {definition && (
        <p
          data-testid={`containers-group-definition-${groupKey(project)}`}
          className="truncate font-mono text-[10px] text-muted-foreground"
          title={definition}
        >
          {definition}
        </p>
      )}
    </button>
  )
}

function ContainerRow({
  container,
  selected,
  onSelect,
}: {
  container: Container
  selected: boolean
  onSelect: (selection: Selection) => void
}) {
  const ports = container.ports
    .map((p) => `${p.host_port === null ? '' : `${p.host_port}→`}${p.container_port}/${p.protocol}`)
    .join(' ')
  return (
    // The badge sits beside the row rather than inside it: a cluster link nested in the row's own
    // button would be a button inside a button, and the keyboard would never reach it.
    <div className="flex h-full w-full min-w-0 items-center border-b border-border/40 pr-2">
      {/* A button for the same reason the heading is one: choosing a container is what opens its
          verbs, and the only route to a mutation must be reachable from the keyboard. */}
      <button
        type="button"
        aria-pressed={selected}
        onClick={() => {
          onSelect({ kind: 'container', id: container.id })
        }}
        data-testid={`container-row-${container.name}`}
        data-state={container.state}
        data-selected={selected ? 'true' : 'false'}
        className={cn(
          'flex h-full min-w-0 flex-1 items-center gap-2 px-3 text-left text-xs',
          selected && 'bg-muted',
        )}
      >
        <span
          aria-hidden="true"
          data-testid={`container-state-${container.name}`}
          className={cn(
            'size-1.5 shrink-0 rounded-full',
            container.state === 'running' && 'bg-status-ok',
            container.state === 'restarting' && 'bg-status-warn',
            container.state !== 'running' &&
              container.state !== 'restarting' &&
              'bg-muted-foreground',
          )}
        />
        <span className="min-w-0 flex-1 truncate font-mono">{container.name}</span>
        {/* Image and ports are the first things to go at phone width: the name and whether it is up
          are what the scene is for, and a row that scrolls sideways answers neither (FR-021). */}
        <span className="hidden min-w-0 flex-1 truncate text-muted-foreground sm:inline">
          {container.image}
        </span>
        <span className="hidden shrink-0 font-mono text-muted-foreground md:inline">{ports}</span>
        <span className="shrink-0 truncate text-muted-foreground">{container.status}</span>
      </button>
      {container.cluster !== null && (
        <ClusterLinkBadge container={container.name} cluster={container.cluster} />
      )}
    </div>
  )
}
