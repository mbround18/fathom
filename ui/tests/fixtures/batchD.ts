/** Batch D list fixtures and read replies (HPA, PriorityClass, RuntimeClass, Lease, webhooks). */
import type { ResourceListEvent } from '../../src/protocol/types'

export const HPA_ROW = {
  namespace: 'default',
  name: 'web',
  target_ref: 'Deployment/web',
  min_replicas: 2,
  max_replicas: 10,
  current_replicas: 4,
}

export const HPA_DETAIL = {
  ...HPA_ROW,
  creation_timestamp: '2024-01-01T00:00:00Z',
  labels: { app: 'web' },
  annotations: {},
  desired_replicas: 6,
  conditions: ['AbleToScale=True (SucceededGetScale)', 'ScalingActive=True (ValidMetricFound)'],
  raw_json: JSON.stringify({
    spec: {
      metrics: [
        {
          type: 'Resource',
          resource: { name: 'cpu', target: { type: 'Utilization', averageUtilization: 70 } },
        },
      ],
    },
    status: {
      currentMetrics: [
        { type: 'Resource', resource: { name: 'cpu', current: { averageUtilization: 85 } } },
      ],
    },
  }),
}

export const PRIORITY_ROW = {
  name: 'high-priority',
  value: 1000000,
  global_default: false,
  description: 'Critical workloads',
}
export const RUNTIME_ROW = { name: 'gvisor', handler: 'runsc' }
export const LEASE_ROW = {
  namespace: 'kube-system',
  name: 'kube-scheduler',
  holder_identity: 'node-1_abc',
  renew_time: '2024-01-01T00:00:00Z',
}
export const MUTATING_ROW = { name: 'istio-sidecar-injector', webhooks_count: 1 }
export const VALIDATING_ROW = { name: 'gatekeeper-validating', webhooks_count: 1 }

export const MUTATING_DETAIL = {
  name: MUTATING_ROW.name,
  creation_timestamp: '2024-01-01T00:00:00Z',
  labels: {},
  annotations: {},
  webhooks: [{ name: 'inject.istio.io', failure_policy: 'Fail', side_effects: 'None' }],
  raw_json: JSON.stringify({
    webhooks: [
      {
        name: 'inject.istio.io',
        clientConfig: { service: { namespace: 'istio-system', name: 'istiod', path: '/inject' } },
        rules: [
          { operations: ['CREATE'], apiGroups: [''], apiVersions: ['v1'], resources: ['pods'] },
        ],
      },
    ],
  }),
}

export const VPA_ROW = {
  namespace: 'default',
  name: 'web-vpa',
  target_ref: 'Deployment/web',
  update_mode: 'Auto',
  recommendations: [
    {
      container: 'app',
      lower_bound: { cpu: '25m', memory: '100Mi' },
      target: { cpu: '50m', memory: '200Mi' },
      upper_bound: { cpu: '1', memory: '1Gi' },
    },
  ],
}

export const VPA_DETAIL = {
  ...VPA_ROW,
  creation_timestamp: '2024-01-01T00:00:00Z',
  labels: { app: 'web' },
  annotations: {},
  conditions: ['RecommendationProvided=True (Ok)'],
  raw_json: JSON.stringify({ spec: { targetRef: { kind: 'Deployment', name: 'web' } } }),
}

export const BATCH_D_LIST_EVENTS: Record<string, ResourceListEvent> = {
  'config/horizontalpodautoscalers': {
    type: 'HorizontalPodAutoscalersUpdated',
    data: [HPA_ROW],
  },
  'config/verticalpodautoscalers': { type: 'VerticalPodAutoscalersUpdated', data: [VPA_ROW] },
  'config/priorityclasses': { type: 'PriorityClassesUpdated', data: [PRIORITY_ROW] },
  'config/runtimeclasses': { type: 'RuntimeClassesUpdated', data: [RUNTIME_ROW] },
  'config/leases': { type: 'LeasesUpdated', data: [LEASE_ROW] },
  'config/mutatingwebhooks': {
    type: 'MutatingWebhookConfigurationsUpdated',
    data: [MUTATING_ROW],
  },
  'config/validatingwebhooks': {
    type: 'ValidatingWebhookConfigurationsUpdated',
    data: [VALIDATING_ROW],
  },
}
