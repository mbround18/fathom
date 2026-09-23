/**
 * `ContainerImageInspected`/`ContainerImageInspectFailed` (010-container-image-inspection):
 * keyed by `${namespace}/${pod}/${container}` rather than the generic `details` keying (whose
 * `readKeyOf` assumes a `name` field the image-inspection payload doesn't carry).
 */
import type { ResourceReadEvent } from '@/protocol/types'

import type { Draft } from '../draft'

export function imageInspectKey(namespace: string, pod: string, container: string): string {
  return `${namespace}/${pod}/${container}`
}

type Inspected = Extract<ResourceReadEvent, { type: 'ContainerImageInspected' }>
type InspectFailed = Extract<ResourceReadEvent, { type: 'ContainerImageInspectFailed' }>

export function handleImageInspect(d: Draft, event: Inspected | InspectFailed): void {
  const { namespace, pod, container } = event.data
  const key = imageInspectKey(namespace, pod, container)
  d.edit('imageInspections')[key] =
    event.type === 'ContainerImageInspected'
      ? { status: 'ready', inspection: event.data.inspection, updatedAt: Date.now() }
      : { status: 'error', error: event.data.error, updatedAt: Date.now() }
}
