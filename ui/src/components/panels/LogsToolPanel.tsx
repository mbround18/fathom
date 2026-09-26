/**
 * Logs tool: pick any pod in the active namespace scope and tail it, without first navigating
 * to Pods and opening the inspector. Reuses the inspector's `PodLogsTab` (same stream plumbing);
 * that tab needs the pod's details, so choosing a pod also sends its `SelectPod`.
 */
import { useEffect, useMemo, useState } from 'react'

import type { PodSummary } from '@/protocol/types'
import { listKey, useStore } from '@/store'

import { Picker } from '../Picker'
import { Empty, Loading, NoKubeconfig } from '../shell/PaneStates'
import { PodLogsTab } from './PodLogsTab'

const PODS = 'workloads/pods'

export default function LogsToolPanel() {
  const context = useStore((s) => s.session.activeContext)
  const namespace = useStore((s) => s.session.activeNamespace)
  const list = useStore((s) => s.lists[listKey(context, PODS, 'Namespaced', null)])
  const refreshSection = useStore((s) => s.refreshSection)
  const inspect = useStore((s) => s.inspect)
  const [picked, setPicked] = useState<string | undefined>(undefined)

  // The pod list is otherwise only fetched by the Pods pane.
  useEffect(() => {
    if (context !== undefined && list === undefined) refreshSection(PODS)
  }, [context, list, refreshSection])

  const pods = useMemo(() => {
    const rows = (list?.eventType === 'PodsUpdated' ? list.rows : []) as readonly PodSummary[]
    return rows
      .filter((p) => namespace === null || p.namespace === namespace)
      .map((p) => ({ value: `${p.namespace}/${p.name}`, label: `${p.namespace}/${p.name}` }))
      .sort((a, b) => a.label.localeCompare(b.label))
  }, [list, namespace])

  // A pick that left the scope (namespace switch, pod deleted) no longer applies.
  const current = picked && pods.some((p) => p.value === picked) ? picked : undefined
  const [ns, name] = current ? (current.split('/') as [string, string]) : []

  if (context === undefined) return <NoKubeconfig />
  if (list === undefined || (list.status === 'loading' && list.rows.length === 0)) {
    return <Loading rows={4} label="Loading pods" />
  }

  return (
    <div data-testid="logs-tool" className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-border p-2">
        <Picker
          label="Pod"
          prefix="pod"
          value={current}
          options={pods}
          placeholder={pods.length === 0 ? 'No pods' : 'Choose a pod'}
          disabled={pods.length === 0}
          onChange={(v) => {
            setPicked(v)
            const [pns, pname] = v.split('/') as [string, string]
            inspect('Pod', pns, pname)
          }}
        />
      </div>
      <div className="min-h-0 flex-1">
        {pods.length === 0 ? (
          <Empty
            title="No pods"
            detail={
              namespace === null
                ? 'This cluster has no pods to stream logs from.'
                : `No pods in namespace ${namespace}. Pick another namespace or All.`
            }
          />
        ) : ns && name ? (
          <PodLogsTab key={current} target={{ section: PODS, kind: 'Pod', namespace: ns, name }} />
        ) : (
          <Empty title="Choose a pod" detail="Select a pod to stream logs." />
        )}
      </div>
    </div>
  )
}
