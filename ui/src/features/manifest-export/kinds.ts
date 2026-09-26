/**
 * 040-manifest-export: which listed kinds can be exported, and their API identity. The backend
 * resolves the resource by discovery, so this table only has to name apiVersion and kind.
 * Synthetic rows (Argo Applications, Helm releases, Flux and cert chains) are not Kubernetes
 * objects of that kind and are excluded (research R1).
 */
import type { ManifestObjectRef } from '@/protocol/types'
import type { AnyDescriptor } from '@/resources'
import type { ListRow } from '@/store'

interface ApiIdentity {
  apiVersion: string
  /** The real kind, where the descriptor's label differs (`Endpoint` → `Endpoints`). */
  kind?: string
}

export const EXPORTABLE_KINDS: Readonly<Record<string, ApiIdentity>> = {
  Pod: { apiVersion: 'v1' },
  Service: { apiVersion: 'v1' },
  ConfigMap: { apiVersion: 'v1' },
  Secret: { apiVersion: 'v1' },
  Namespace: { apiVersion: 'v1' },
  Node: { apiVersion: 'v1' },
  PersistentVolume: { apiVersion: 'v1' },
  PersistentVolumeClaim: { apiVersion: 'v1' },
  ServiceAccount: { apiVersion: 'v1' },
  ReplicationController: { apiVersion: 'v1' },
  ResourceQuota: { apiVersion: 'v1' },
  LimitRange: { apiVersion: 'v1' },
  Endpoint: { apiVersion: 'v1', kind: 'Endpoints' },
  Endpoints: { apiVersion: 'v1' },
  Event: { apiVersion: 'v1' },
  Deployment: { apiVersion: 'apps/v1' },
  StatefulSet: { apiVersion: 'apps/v1' },
  DaemonSet: { apiVersion: 'apps/v1' },
  ReplicaSet: { apiVersion: 'apps/v1' },
  ControllerRevision: { apiVersion: 'apps/v1' },
  Job: { apiVersion: 'batch/v1' },
  CronJob: { apiVersion: 'batch/v1' },
  HorizontalPodAutoscaler: { apiVersion: 'autoscaling/v2' },
  VerticalPodAutoscaler: { apiVersion: 'autoscaling.k8s.io/v1' },
  Ingress: { apiVersion: 'networking.k8s.io/v1' },
  IngressClass: { apiVersion: 'networking.k8s.io/v1' },
  NetworkPolicy: { apiVersion: 'networking.k8s.io/v1' },
  EndpointSlice: { apiVersion: 'discovery.k8s.io/v1' },
  StorageClass: { apiVersion: 'storage.k8s.io/v1' },
  CSIDriver: { apiVersion: 'storage.k8s.io/v1' },
  CSINode: { apiVersion: 'storage.k8s.io/v1' },
  VolumeAttachment: { apiVersion: 'storage.k8s.io/v1' },
  Role: { apiVersion: 'rbac.authorization.k8s.io/v1' },
  ClusterRole: { apiVersion: 'rbac.authorization.k8s.io/v1' },
  RoleBinding: { apiVersion: 'rbac.authorization.k8s.io/v1' },
  ClusterRoleBinding: { apiVersion: 'rbac.authorization.k8s.io/v1' },
  PodDisruptionBudget: { apiVersion: 'policy/v1' },
  PriorityClass: { apiVersion: 'scheduling.k8s.io/v1' },
  RuntimeClass: { apiVersion: 'node.k8s.io/v1' },
  Lease: { apiVersion: 'coordination.k8s.io/v1' },
  MutatingWebhookConfiguration: { apiVersion: 'admissionregistration.k8s.io/v1' },
  ValidatingWebhookConfiguration: { apiVersion: 'admissionregistration.k8s.io/v1' },
  CustomResourceDefinition: { apiVersion: 'apiextensions.k8s.io/v1' },
  APIService: { apiVersion: 'apiregistration.k8s.io/v1' },
}

/** Descriptor kinds that are deliberately not exportable (not a Kubernetes object of that kind). */
export const NON_EXPORTABLE_KINDS: ReadonlySet<string> = new Set([
  'Application',
  'HelmRelease',
  'FluxResource',
  'CertificateChainStage',
])

export function isExportable(descriptor: AnyDescriptor | undefined): boolean {
  return descriptor !== undefined && Object.hasOwn(EXPORTABLE_KINDS, descriptor.kind)
}

/** The API identity of one listed row, or undefined when its kind is not exportable. */
export function manifestRefFor(
  descriptor: AnyDescriptor,
  row: ListRow,
): ManifestObjectRef | undefined {
  const id = EXPORTABLE_KINDS[descriptor.kind]
  if (!id) return undefined
  const ref = descriptor.ref(row)
  return {
    api_version: id.apiVersion,
    kind: id.kind ?? descriptor.kind,
    namespace: descriptor.namespaced ? (ref.namespace ?? null) : null,
    name: ref.name,
  }
}
