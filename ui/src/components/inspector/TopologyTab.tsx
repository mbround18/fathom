/**
 * 043 US6 (FR-031/FR-032): the inspector's per-resource Topology tab. It shows the inspected
 * resource with its upstream neighbours (what owns, selects, scales, mounts or reconciles it)
 * above and its downstream neighbours below, from the existing read-only related-resources
 * resolver (`RefreshRelatedResources`, spec 004). "Follow" re-roots the inspector on a
 * neighbour through its back history; the store keeps this tab selected across the drill.
 *
 * Expand/collapse in place and click-through inside the whole-cluster graph are feedback #23.
 */
import { ArrowDownIcon, ArrowUpIcon } from '@phosphor-icons/react'
import { useEffect, useMemo } from 'react'

import { StatusBadge } from '@/components/StatusBadge'
import { Button } from '@companion/ui'
import type { RelatedResourceRef } from '@/protocol/types'
import { relatedObjectRef } from '@/resources/related'
import type { ObjectRef } from '@/resources'
import { detailKey, useStore } from '@/store'
import { splitRelated, type Neighbour } from '@/topology/resource-topology'

import { Empty, Loading } from '@companion/terminal'

interface RelatedPayload {
  kind: string
  namespace: string | null
  name: string
  related: RelatedResourceRef[]
}

export function TopologyTab({ target }: { target: ObjectRef }) {
  const send = useStore((s) => s.send)
  const ctx = useStore((s) => s.session.activeContext)
  const state = useStore(
    (s) => s.details[detailKey(ctx, 'RelatedResources', target.namespace, target.name)],
  )

  // One request per root; the inspector remounts this tab when its target changes.
  useEffect(() => {
    send({
      type: 'RefreshRelatedResources',
      data: {
        section: target.section,
        kind: target.kind,
        namespace: target.namespace,
        name: target.name,
      },
    })
  }, [send, target.section, target.kind, target.namespace, target.name])

  // The reply is keyed by namespace/name only, so a same-named resource of another kind (the
  // root this tab showed before a Follow, say) can sit there: discard anything not for `target`.
  const payload =
    state?.eventType === 'RelatedResourcesUpdated' ? (state.data as RelatedPayload) : undefined
  const current =
    payload !== undefined &&
    payload.kind === target.kind &&
    payload.name === target.name &&
    (payload.namespace ?? null) === target.namespace
      ? payload.related
      : undefined

  const topology = useMemo(
    () => (current === undefined ? undefined : splitRelated(target, current)),
    [current, target],
  )

  if (!topology) return <Loading rows={4} label="Loading topology" />
  const none = topology.upstream.length === 0 && topology.downstream.length === 0

  return (
    <div data-testid="inspector-topology" className="flex flex-col gap-3 p-3 text-sm">
      {!none && (
        <NeighbourSection
          title="Upstream"
          hint="What owns, selects, scales or mounts it"
          icon="up"
          neighbours={topology.upstream}
        />
      )}
      <div
        data-testid="topology-root"
        aria-label={`${target.kind} ${target.name}`}
        className="flex min-w-0 flex-wrap items-baseline gap-x-2 border border-accent-neon/60 bg-accent-neon/5 px-3 py-2 font-mono text-xs"
      >
        <span className="text-accent-neon">{target.kind}</span>
        <span className="min-w-0 truncate text-foreground">
          {target.namespace ? `${target.namespace}/` : ''}
          {target.name}
        </span>
      </div>
      {!none && (
        <NeighbourSection
          title="Downstream"
          hint="What it owns, selects, scales or runs on"
          icon="down"
          neighbours={topology.downstream}
        />
      )}
      {none && (
        <Empty
          title="No related resources"
          detail={`Nothing in the cluster links to ${target.name} right now.`}
        />
      )}
    </div>
  )
}

function NeighbourSection({
  title,
  hint,
  icon,
  neighbours,
}: {
  title: string
  hint: string
  icon: 'up' | 'down'
  neighbours: readonly Neighbour[]
}) {
  const id = `inspector-topology-${title.toLowerCase()}`
  const Icon = icon === 'up' ? ArrowUpIcon : ArrowDownIcon
  return (
    <section aria-labelledby={id} className="flex flex-col gap-1">
      <h3
        id={id}
        className="flex items-center gap-1.5 font-display text-xs tracking-wider text-muted-foreground uppercase"
      >
        <Icon aria-hidden="true" className="size-3.5" />
        {title}
        <span className="font-sans tracking-normal normal-case">· {hint}</span>
      </h3>
      {neighbours.length === 0 ? (
        <p className="px-1 text-xs text-muted-foreground">None</p>
      ) : (
        <ul className="flex flex-col">
          {neighbours.map((n, i) => (
            <NeighbourRow
              key={`${n.kind}-${n.namespace ?? ''}-${n.name}-${n.relation}-${String(i)}`}
              n={n}
            />
          ))}
        </ul>
      )}
    </section>
  )
}

function NeighbourRow({ n }: { n: Neighbour }) {
  const openInspector = useStore((s) => s.openInspector)
  const label = `${n.kind} ${n.namespace ? `${n.namespace}/` : ''}${n.name}`
  return (
    <li
      aria-label={label}
      className="flex min-w-0 items-center gap-2 border-b border-border/50 px-1 py-1 text-xs"
    >
      <span className="w-24 shrink-0 truncate text-muted-foreground">{n.relationLabel}</span>
      <span className="shrink-0 font-mono text-accent-neon">{n.kind}</span>
      <span className="min-w-0 flex-1 truncate font-mono">
        {n.namespace ? `${n.namespace}/` : ''}
        {n.name}
      </span>
      {n.status_hint && (
        <span className="shrink-0">
          <StatusBadge status={n.status_hint} />
        </span>
      )}
      <Button
        type="button"
        variant="outline"
        size="sm"
        aria-label={`Follow ${label}`}
        className="min-h-[var(--touch-target)] shrink-0"
        onClick={() => {
          // No source list: the neighbour is usually not a row of the list this came from.
          openInspector(relatedObjectRef(n), undefined, true)
        }}
      >
        Follow
      </Button>
    </li>
  )
}
