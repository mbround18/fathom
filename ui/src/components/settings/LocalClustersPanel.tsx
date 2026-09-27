/**
 * 043 US3: Settings › Local clusters. Disposable kind clusters on this computer (030), split out
 * of Cluster settings. Desktop only: hosted mode has no local machine to create them on.
 */
import { KindClusterPanel } from '@/components/panels/KindClusterPanel'
import { Empty } from '@/components/shell/PaneStates'
import { useStore } from '@/store'

export function LocalClustersPanel() {
  const hosted = useStore((s) => s.hosted.mode === 'hosted')
  if (hosted) {
    return (
      <Empty
        title="Local clusters are not available"
        detail="Local kind clusters can only be created from the desktop app."
      />
    )
  }
  return (
    <div
      data-testid="local-clusters-panel"
      className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-4"
    >
      <KindClusterPanel />
    </div>
  )
}
