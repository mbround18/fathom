/**
 * Whether an optional, CRD-backed integration is present in the active cluster, from the last
 * scan the store holds. `undefined` = not scanned yet (never render that as "not installed").
 */
import { detectArgo } from '@/features/argo/argo'
import type { SectionKey } from '@/protocol/types'
import type { StoreData } from '@/store'

export const NOT_INSTALLED_HINT = 'Not installed in this cluster'

type Slices = Pick<
  StoreData,
  | 'flux'
  | 'knative'
  | 'keda'
  | 'velero'
  | 'certManager'
  | 'hubble'
  | 'gatekeeper'
  | 'trivy'
  | 'crds'
  | 'metrics'
>

export function integrationInstalled(s: Slices, key: SectionKey): boolean | undefined {
  switch (key) {
    case 'integrations/flux':
      return s.flux.overview?.installed
    case 'integrations/knative':
      return s.knative.overview?.installed
    case 'integrations/keda':
      return s.keda.overview?.installed
    case 'integrations/velero':
      return s.velero.overview?.installed
    case 'integrations/cert-manager':
      return s.certManager.overview?.installed
    case 'integrations/hubble':
      return s.hubble.summary?.installed
    case 'integrations/vulnerabilities':
      return s.trivy.overview?.installed
    case 'integrations/gatekeeper': {
      const st = s.gatekeeper.overview?.state
      return st === undefined ? undefined : st !== 'NotInstalled'
    }
    // Argo CD has no dedicated backend: its presence is the argoproj.io CRDs in the catalog.
    case 'integrations/argo':
      return s.crds.catalogLoaded ? detectArgo(s.crds.catalog).found.length > 0 : undefined
    // Metrics and Grafana come from the metrics-stack service scan (`RefreshIntegrations`).
    case 'integrations/metrics': {
      const scan = s.metrics.stackScan
      return scan === undefined ? undefined : scan.tier !== 'None'
    }
    case 'integrations/grafana': {
      const scan = s.metrics.stackScan
      return scan === undefined ? undefined : scan.components.some((c) => c.kind === 'Grafana')
    }
    default:
      return undefined
  }
}
