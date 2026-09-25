/**
 * `RbacSelfAccessUpdated`/`NamespaceAccessResolved`/`KubeadmDetectionUpdated`
 * (011-rbac-oidc-administration): three read-only RBAC payloads with no `name` field, so they
 * bypass the generic `details` keying (`readKeyOf` assumes one) the same way
 * `handlers/imageInspect.ts` does for image inspections — see `store/index.ts`'s `rbacAccess`
 * slice doc comment.
 */
import type { RbacAccessCheck, RbacAccessResult, ResourceReadEvent } from '@/protocol/types'

import type { Draft } from '../draft'

export type RbacAccessEvent = Extract<
  ResourceReadEvent,
  {
    type:
      | 'RbacSelfAccessUpdated'
      | 'ResourceMutationAccessChecked'
      | 'NamespaceAccessResolved'
      | 'KubeadmDetectionUpdated'
  }
>

/** Merge key for one access check: `(verb, group, resource, namespace)`. */
export function accessKey(c: RbacAccessCheck): string {
  return [c.verb, c.group ?? '', c.resource, c.namespace ?? ''].join('|')
}

/** Key for `rbacAccess.resourceAccess` (YAML apply/create checks resolved by discovery). */
export function resourceAccessKey(
  apiVersion: string,
  kind: string,
  namespace: string | null,
  verb: string,
): string {
  return [apiVersion, kind, namespace ?? '', verb].join('|')
}

/**
 * Merge incoming results into the existing ones by check key (last write wins per key) so
 * concurrent panels' checks never clobber each other (018 research Decision 4).
 */
export function mergeAccessResults(
  existing: RbacAccessResult[] | undefined,
  incoming: RbacAccessResult[],
): RbacAccessResult[] {
  const byKey = new Map<string, RbacAccessResult>()
  for (const r of existing ?? []) byKey.set(accessKey(r.check), r)
  for (const r of incoming) byKey.set(accessKey(r.check), r)
  return [...byKey.values()]
}

export function handleRbacAccess(d: Draft, event: RbacAccessEvent): void {
  switch (event.type) {
    case 'RbacSelfAccessUpdated':
      d.edit('rbacAccess').selfAccess = mergeAccessResults(
        d.edit('rbacAccess').selfAccess,
        event.data.results,
      )
      return
    case 'ResourceMutationAccessChecked': {
      const { api_version, kind, result } = event.data
      const slice = d.edit('rbacAccess')
      slice.resourceAccess = {
        ...slice.resourceAccess,
        [resourceAccessKey(api_version, kind, result.check.namespace, result.check.verb)]: result,
      }
      return
    }
    case 'NamespaceAccessResolved':
      d.edit('rbacAccess').namespaceAccess = event.data
      return
    case 'KubeadmDetectionUpdated':
      d.edit('rbacAccess').kubeadmDetection = event.data
      return
  }
}
