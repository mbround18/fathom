/**
 * 018-rbac-aware-mutation-ui: shared point-lookup for "can I do verb V on resource R right now".
 * Dispatches a `RefreshRbacSelfAccess` (live `SelfSubjectAccessReview`) for a check that has no
 * cached answer yet and reads the answer from the merged `rbacAccess.selfAccess` slice.
 *
 * Advisory only (FR-005): the API server still enforces. While the answer is unknown (in flight,
 * never requested, or no check supplied) the status is `'unknown'` and callers must treat it as
 * enabled and show no tooltip (FR-006).
 */
import { useEffect } from 'react'

import type { RbacAccessCheck, RbacAccessResult } from '@/protocol/types'
import { useStore } from '@/store'
import { accessKey, resourceAccessKey } from '@/store/handlers/rbacAccess'

export type MutationAccessStatus = 'unknown' | 'allowed' | 'denied'

export interface MutationAccess {
  status: MutationAccessStatus
  /** The API server's reason for a denial, if it gave one. */
  reason?: string
}

/** Check for a resource with a statically known group/plural (nodes, secrets, bindings). */
export interface AccessCheckInput {
  verb: string
  resource: string
  /** API group; `''` (default) is the core group. */
  group?: string
  namespace?: string | null
}

export const RBAC_GROUP = 'rbac.authorization.k8s.io'

export function toAccess(result: RbacAccessResult | undefined): MutationAccess {
  if (!result) return { status: 'unknown' }
  if (result.allowed) return { status: 'allowed' }
  return result.reason ? { status: 'denied', reason: result.reason } : { status: 'denied' }
}

function normalize(c: AccessCheckInput): RbacAccessCheck {
  return {
    verb: c.verb,
    resource: c.resource,
    group: c.group ?? '',
    namespace: c.namespace ?? null,
  }
}

/** Checks already dispatched this session (per context), so N rows mounting at once send one. */
const inFlight = new Set<string>()

export function useMutationAccess(input: AccessCheckInput | undefined): MutationAccess {
  const send = useStore((s) => s.send)
  const connected = useStore((s) => s.session.activeContext)
  const check = input ? normalize(input) : undefined
  const key = check ? accessKey(check) : undefined
  const result = useStore((s) =>
    key === undefined
      ? undefined
      : s.rbacAccess.selfAccess?.find((r) => accessKey(r.check) === key),
  )
  const known = result !== undefined

  useEffect(() => {
    // Re-check when the context changes; skip when the answer is already cached.
    if (!check || known || key === undefined) return
    const flightKey = `${connected ?? ''}|${key}`
    if (inFlight.has(flightKey)) return
    inFlight.add(flightKey)
    send({ type: 'RefreshRbacSelfAccess', data: { checks: [check] } })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, connected, known])

  return toAccess(result)
}

/** YAML apply/create: the backend resolves the real plural/group by discovery. */
export function useResourceMutationAccess(
  input: { apiVersion: string; kind: string; namespace: string | null; verb: string } | undefined,
): MutationAccess {
  const send = useStore((s) => s.send)
  const key = input
    ? resourceAccessKey(input.apiVersion, input.kind, input.namespace, input.verb)
    : undefined
  const result = useStore((s) =>
    key === undefined ? undefined : s.rbacAccess.resourceAccess?.[key],
  )
  const known = result !== undefined

  useEffect(() => {
    if (!input || known) return
    send({
      type: 'CheckResourceMutationAccess',
      data: {
        api_version: input.apiVersion,
        kind: input.kind,
        namespace: input.namespace,
        verb: input.verb,
      },
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, known])

  return toAccess(result)
}

/** Tooltip text for a denied action, or `undefined` when nothing should be shown. */
export function deniedTitle(access: MutationAccess, what: string): string | undefined {
  if (access.status !== 'denied') return undefined
  return access.reason
    ? `You are not permitted to ${what}: ${access.reason}`
    : `You are not permitted to ${what}.`
}
