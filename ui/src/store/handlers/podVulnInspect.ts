/**
 * `PodVulnerabilitiesInspected`/`PodVulnerabilitiesInspectFailed` (021-trivy-vulnerability-pane):
 * keyed by `${namespace}/${pod}` rather than the generic `details` keying.
 */
import type { ResourceReadEvent } from '@/protocol/types'

import type { Draft } from '../draft'

export function podVulnKey(namespace: string, pod: string): string {
  return `${namespace}/${pod}`
}

type Inspected = Extract<ResourceReadEvent, { type: 'PodVulnerabilitiesInspected' }>
type InspectFailed = Extract<ResourceReadEvent, { type: 'PodVulnerabilitiesInspectFailed' }>

export function handlePodVulnInspect(d: Draft, event: Inspected | InspectFailed): void {
  const key = podVulnKey(event.data.namespace, event.data.pod)
  d.edit('podVulnInspections')[key] =
    event.type === 'PodVulnerabilitiesInspected'
      ? { status: 'ready', data: event.data.inspection }
      : { status: 'error', error: event.data.error }
}
