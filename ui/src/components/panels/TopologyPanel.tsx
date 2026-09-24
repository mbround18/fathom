/**
 * Topology Graph View tool pane (007-topology-graph-view, FR-001/FR-015). Owns the
 * connected/disconnected/no-kubeconfig framing (same contract as `ListBody` in `PaneHost.tsx`)
 * and, on mount, refreshes every section the graph reads from so the first paint isn't stuck on
 * `loading` for data the operator never explicitly asked for (namespace/app/helm/node tiers plus
 * the always-visible workload tier).
 */
import { useEffect } from 'react'

import { applications } from '@/resources/applications'
import { daemonsets } from '@/resources/daemonsets'
import { deployments } from '@/resources/deployments'
import { helm } from '@/resources/helm'
import { nodes } from '@/resources/nodes'
import { statefulsets } from '@/resources/statefulsets'
import { useStore, type PaneId } from '@/store'

import { TopologyCanvas } from '../topology/TopologyCanvas'
import { Disconnected, NoKubeconfig } from '../shell/PaneStates'

const SECTIONS = [applications, helm, nodes, deployments, statefulsets, daemonsets].map(
  (d) => d.section,
)

export function TopologyPanel({ paneId }: { paneId: PaneId }) {
  const disconnected = useStore((s) => s.conn.status === 'backend-gone')
  const fatal = useStore((s) => s.conn.fatal)
  const noKubeconfig = useStore((s) => s.session.contextsLoaded && s.session.contexts.length === 0)
  const notice = useStore((s) => s.conn.lastNotice?.message)
  const refreshSection = useStore((s) => s.refreshSection)

  useEffect(() => {
    for (const section of SECTIONS) refreshSection(section)
  }, [refreshSection])

  if (disconnected) return <Disconnected detail={fatal} />
  if (noKubeconfig) return <NoKubeconfig detail={notice} />

  return (
    <div className="flex h-full min-h-0 w-full min-w-0 flex-col" data-testid="topology-panel">
      <TopologyCanvas paneId={paneId} />
    </div>
  )
}
