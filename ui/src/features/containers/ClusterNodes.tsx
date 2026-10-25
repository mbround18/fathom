/**
 * 048-docker-management US4: the other direction of the cross-link — a cluster's own containers.
 *
 * The containers scene says which cluster a node backs (`ClusterLinkBadge`); this says which
 * containers back a cluster, from the same `ClusterLink` the backend derived from the cluster
 * label. One source, two readings, so the two sides cannot disagree about what a cluster is made
 * of (FR-031).
 *
 * It also refuses to let the cluster read as healthy when a node is not running. `kind get
 * clusters` lists a cluster whose nodes are all stopped exactly as it lists a working one, so a
 * node someone stopped outside Fathom would otherwise leave the row claiming a cluster that
 * cannot answer (FR-032).
 */
import { Button } from '@companion/ui'
import { SECTION } from '@/protocol/commands'
import type { Container } from '@/protocol/types'
import { useStore } from '@/store'

/** The containers this cluster's link names, resolved against the inventory on screen. */
function nodesOf(
  cluster: string,
  links: { cluster: string; container_ids: string[] }[],
  all: Container[],
) {
  const ids = links.find((l) => l.cluster === cluster)?.container_ids ?? []
  return ids.flatMap((id) => all.filter((c) => c.id === id))
}

export function ClusterNodes({ cluster }: { cluster: string }) {
  const links = useStore((s) => s.containerRuntime.clusterLinks)
  const inventory = useStore((s) => s.containerRuntime.inventory)
  const navigate = useStore((s) => s.navigate)

  const all = (inventory?.projects ?? []).flatMap((p) => p.containers)
  const nodes = nodesOf(cluster, links, all)
  // No link at all means the containers scene has not read the daemon yet — a different thing
  // from a cluster with no nodes, and saying "0 nodes" for it would be a claim Fathom cannot make.
  if (links.every((l) => l.cluster !== cluster)) return null

  const down = nodes.filter((c) => c.state !== 'running')

  return (
    <div data-testid={`kind-nodes-${cluster}`} className="flex flex-col gap-1 pl-4 text-[11px]">
      <ul className="flex flex-col gap-0.5">
        {nodes.map((c) => (
          <li
            key={c.id}
            data-testid={`kind-node-${c.name}`}
            data-state={c.state}
            className="flex min-w-0 items-baseline gap-2"
          >
            <span className="truncate font-mono">{c.name}</span>
            <span className="shrink-0 text-muted-foreground">{c.status}</span>
          </li>
        ))}
      </ul>
      {down.length > 0 && (
        <p data-testid={`kind-nodes-degraded-${cluster}`} className="text-status-warn">
          {down.length} of {nodes.length} {nodes.length === 1 ? 'node is' : 'nodes are'} not running
          — this cluster cannot be relied on until they are.
        </p>
      )}
      <div>
        <Button
          size="sm"
          variant="ghost"
          data-testid={`kind-open-containers-${cluster}`}
          onClick={() => {
            navigate(SECTION.containers)
          }}
        >
          View these containers
        </Button>
      </div>
    </div>
  )
}
