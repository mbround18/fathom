/** `BackendEvent::ResourceRead` arm: one-object reads (details, raw JSON for YAML, metrics). */
import type { BackendEventEnvelope, ResourceReadEvent } from '@/protocol/types'

import type { Draft } from '../draft'
import { applyLiveUpdate } from '../editSession'
import { detailKey, detailKind, FLUX_RESOURCE_SECTION } from '../keys'

/** `PodDetailsUpdated` → `Pod`; `PodMetricsHistoryUpdated` → `PodMetricsHistory`. */
export function readKind(type: ResourceReadEvent['type']): string {
  return type.replace(/(Details)?Updated$/, '')
}

/**
 * Every `ResourceReadEvent` variant except the two 010-container-image-inspection ones,
 * 013-agent-sdk-ops's `AgentConversationUpdated`, and 011-rbac-oidc-administration's
 * no-`name`-field variants, which `ingest.ts` routes to `handleImageInspect`/
 * `handleAgentConversation`/`handleRbacAccess` before this is ever reached (none of their
 * payloads have a top-level `name` field for the generic `details` keying below to key on).
 */
type GenericReadEvent = Exclude<
  ResourceReadEvent,
  | { type: 'ContainerImageInspected' | 'ContainerImageInspectFailed' | 'AgentConversationUpdated' }
  // 021-trivy-vulnerability-pane: keyed by namespace/pod, routed to `handlePodVulnInspect`.
  | { type: 'PodVulnerabilitiesInspected' | 'PodVulnerabilitiesInspectFailed' }
  // 011-rbac-oidc-administration: no `name` field; routed by `handleRbacAccess` instead.
  | {
      type:
        | 'RbacSelfAccessUpdated'
        | 'ResourceMutationAccessChecked'
        | 'NamespaceAccessResolved'
        | 'KubeadmDetectionUpdated'
        | 'ApiResourcesUpdated'
    }
  // 024-pod-exec-terminal: routed by `handleExecAccess`.
  | { type: 'PodExecAccessChecked' }
>

export function readKeyOf(env: BackendEventEnvelope, event: GenericReadEvent): string {
  const data = event.data
  const ns = 'namespace' in data ? data.namespace : null
  // Applications: `ApplicationDetails.name` is a display name that can differ from `instance`
  // (falls back to a k8s label, see `src/backend/applications.rs`), but the inspector's cache
  // lookup always uses the stable identifier (`ObjectRef.name` = `instance` for Applications,
  // per `applications.tsx`'s `ref()`). Prefer `instance` when present so the two agree.
  const name = 'instance' in data ? data.instance : data.name
  // Flux origin click-through (spec 005): one section covers both Kustomization- and
  // HelmRelease-kind origins, so the cache key must be keyed by the real k8s kind carried in
  // the payload (`data.kind`), not the syntactic `readKind(event.type)` (which would always say
  // "FluxResource" and collide the two kinds under one key) — `flux-resource.tsx`'s `ref()`
  // opens the inspector with `target.kind` set to that same real k8s kind. `detailKind` then
  // namespaces it to the origin's section, so it cannot collide with the object it is the origin
  // *of* — a same-namespace, same-name `HelmRelease` CR and its Helm release (see `keys.ts`).
  const kind =
    (event.type === 'FluxResourceDetailsUpdated' || event.type === 'FluxResourceNotFound') &&
    'kind' in data
      ? detailKind(FLUX_RESOURCE_SECTION, data.kind)
      : readKind(event.type)
  return detailKey(env.route.context, kind, ns, name)
}

export function handleRead(d: Draft, env: BackendEventEnvelope, event: GenericReadEvent): void {
  const key = readKeyOf(env, event)
  const prev = d.read('details')[key]
  const latest = env.correlation.type === 'Request' ? env.correlation.data : prev?.latestRequest
  d.edit('details')[key] = {
    kind: readKind(event.type),
    eventType: event.type,
    data: event.data,
    ...(latest !== undefined ? { latestRequest: latest } : {}),
    updatedAt: Date.now(),
  }

  // T024 (live diff): a watch-driven re-read of the object an edit session is open on
  // re-renders `liveYaml` and flags the diff as stale if the operator already reviewed it.
  const session = d.read('editSessions')[key]
  const raw = 'raw_json' in event.data ? event.data.raw_json : undefined
  if (session && typeof raw === 'string') {
    const next = applyLiveUpdate(session, raw)
    if (next !== session) d.edit('editSessions')[key] = next
  }
}
