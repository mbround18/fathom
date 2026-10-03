/**
 * 048-docker-management US4: the badge that says a container is a cluster node.
 *
 * A kind node is an ordinary container on this daemon, and an operator who does not know that is
 * one mis-click from killing the cluster they are working in (FR-031). So the row says which
 * cluster the container backs, and offers the way to it rather than leaving the operator to
 * recognise a naming pattern.
 *
 * The claim is only ever made from the cluster label the backend read — never from the name. A
 * container called `kind-control-plane` that carries no label gets no badge, because a badge that
 * is sometimes about nothing is a badge the operator learns to ignore.
 */
import { useStore } from '@/store'
import { SECTION } from '@/protocol/commands'

export function ClusterLinkBadge({ container, cluster }: { container: string; cluster: string }) {
  const navigate = useStore((s) => s.navigate)
  return (
    <button
      type="button"
      data-testid={`container-cluster-${container}`}
      data-cluster={cluster}
      title={`${cluster} — open Local clusters`}
      className="shrink-0 border border-accent/60 px-1.5 py-0.5 font-mono text-[10px] tracking-wider text-accent uppercase hover:bg-accent/10"
      onClick={() => {
        navigate(SECTION.localClusters)
      }}
    >
      node of {cluster}
    </button>
  )
}
