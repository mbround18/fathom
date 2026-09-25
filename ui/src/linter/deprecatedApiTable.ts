/**
 * Static, ships-with-the-app table of deprecated/removed apiVersions (upstream Kubernetes
 * deprecation guide). Matched against `metadata.managedFields[].apiVersion` (research.md R7).
 */
export interface DeprecatedApi {
  apiVersion: string
  kinds: readonly string[]
  removedIn: string
  replacement: string
}

export const DEPRECATED_APIS: readonly DeprecatedApi[] = [
  {
    apiVersion: 'extensions/v1beta1',
    kinds: ['Deployment', 'DaemonSet', 'ReplicaSet'],
    removedIn: '1.16',
    replacement: 'apps/v1',
  },
  {
    apiVersion: 'apps/v1beta1',
    kinds: ['Deployment', 'StatefulSet', 'DaemonSet', 'ReplicaSet'],
    removedIn: '1.16',
    replacement: 'apps/v1',
  },
  {
    apiVersion: 'apps/v1beta2',
    kinds: ['Deployment', 'StatefulSet', 'DaemonSet', 'ReplicaSet'],
    removedIn: '1.16',
    replacement: 'apps/v1',
  },
  {
    apiVersion: 'policy/v1beta1',
    kinds: ['PodDisruptionBudget'],
    removedIn: '1.25',
    replacement: 'policy/v1',
  },
  { apiVersion: 'batch/v1beta1', kinds: ['CronJob'], removedIn: '1.25', replacement: 'batch/v1' },
  {
    apiVersion: 'rbac.authorization.k8s.io/v1beta1',
    kinds: ['Role', 'ClusterRole', 'RoleBinding', 'ClusterRoleBinding'],
    removedIn: '1.22',
    replacement: 'rbac.authorization.k8s.io/v1',
  },
  {
    apiVersion: 'rbac.authorization.k8s.io/v1alpha1',
    kinds: ['Role', 'ClusterRole', 'RoleBinding', 'ClusterRoleBinding'],
    removedIn: '1.22',
    replacement: 'rbac.authorization.k8s.io/v1',
  },
  {
    apiVersion: 'autoscaling/v2beta1',
    kinds: ['HorizontalPodAutoscaler'],
    removedIn: '1.25',
    replacement: 'autoscaling/v2',
  },
  {
    apiVersion: 'autoscaling/v2beta2',
    kinds: ['HorizontalPodAutoscaler'],
    removedIn: '1.26',
    replacement: 'autoscaling/v2',
  },
]

export function deprecationFor(apiVersion: string, kind: string): DeprecatedApi | undefined {
  return DEPRECATED_APIS.find((d) => d.apiVersion === apiVersion && d.kinds.includes(kind))
}
