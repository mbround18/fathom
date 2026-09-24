/**
 * 019: always-visible Kubernetes server-version badge beside the context strip. Shows a neutral
 * "Unknown" state before data / when discovery failed, and an amber warning when any node's
 * kubelet version differs from the control plane (skew).
 */
import { useStore } from '@/store'
import { cn } from '@/lib/utils'

type Tone = 'ok' | 'skew' | 'unknown'

const DOT_CLASS: Record<Tone, string> = {
  ok: 'bg-emerald-400',
  skew: 'bg-amber-400',
  unknown: 'bg-muted-foreground/40',
}

export function ClusterVersionBadge() {
  const contexts = useStore((s) => s.session.contexts)
  const cv = useStore((s) => s.clusterVersion)

  if (contexts.length === 0) return null

  const version = cv?.server_version ?? null
  const skew = version !== null && cv?.skew_detected === true
  const tone: Tone = skew ? 'skew' : version ? 'ok' : 'unknown'
  const nodeVersions = cv?.node_kubelet_versions ?? []
  const detail = skew ? `Version skew: server ${version}, nodes ${nodeVersions.join(', ')}` : undefined

  return (
    <div
      data-testid="cluster-version-badge"
      data-server-version={version ?? ''}
      data-skew-detected={skew ? 'true' : 'false'}
      title={detail ?? (version ? `Kubernetes ${version}` : 'Kubernetes version unknown')}
      className="mr-2 flex shrink-0 items-center gap-1.5 rounded-md border border-border/60 bg-background/30 px-2.5 py-1 font-mono text-xs whitespace-nowrap text-muted-foreground"
    >
      <span
        aria-hidden="true"
        data-testid="cluster-version-dot"
        className={cn('h-1.5 w-1.5 shrink-0 rounded-full', DOT_CLASS[tone])}
      />
      <span className={cn(!version && 'italic')}>{version ?? 'Unknown'}</span>
      {skew && (
        <span data-testid="cluster-version-skew-details" className="text-amber-400">
          skew: {nodeVersions.join(', ')}
        </span>
      )}
    </div>
  )
}
