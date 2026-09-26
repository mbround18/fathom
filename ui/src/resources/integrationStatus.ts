/**
 * Whether an optional, CRD-backed integration is present in the active cluster, from the last
 * scan the store holds. `undefined` = not scanned yet (never render that as "not installed").
 */
import type { SectionKey } from '@/protocol/types'
import type { StoreData } from '@/store'

export const NOT_INSTALLED_HINT = 'Not installed in this cluster'

type Slices = Pick<
  StoreData,
  'flux' | 'knative' | 'keda' | 'velero' | 'certManager' | 'hubble' | 'gatekeeper' | 'trivy'
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
    default:
      return undefined
  }
}
