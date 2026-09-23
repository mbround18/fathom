/**
 * `RbacSelfAccessUpdated`/`NamespaceAccessResolved`/`KubeadmDetectionUpdated`
 * (011-rbac-oidc-administration): three read-only RBAC payloads with no `name` field, so they
 * bypass the generic `details` keying (`readKeyOf` assumes one) the same way
 * `handlers/imageInspect.ts` does for image inspections — see `store/index.ts`'s `rbacAccess`
 * slice doc comment.
 */
import type { ResourceReadEvent } from '@/protocol/types'

import type { Draft } from '../draft'

export type RbacAccessEvent = Extract<
  ResourceReadEvent,
  { type: 'RbacSelfAccessUpdated' | 'NamespaceAccessResolved' | 'KubeadmDetectionUpdated' }
>

export function handleRbacAccess(d: Draft, event: RbacAccessEvent): void {
  switch (event.type) {
    case 'RbacSelfAccessUpdated':
      d.edit('rbacAccess').selfAccess = event.data.results
      return
    case 'NamespaceAccessResolved':
      d.edit('rbacAccess').namespaceAccess = event.data
      return
    case 'KubeadmDetectionUpdated':
      d.edit('rbacAccess').kubeadmDetection = event.data
      return
  }
}
