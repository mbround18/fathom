/**
 * Deterministic generators for large, realistic lists (pods-500, pods-10k) and per-object
 * reads. Seeded, so the same fixture is byte-identical on every run.
 */
import type {
  KubernetesEvent,
  PodDetails,
  PodSummary,
  WorkloadSummary,
} from '../../src/protocol/types'

export const NAMESPACES = {
  'kind-a': ['default', 'kube-system', 'monitoring', 'payments', 'checkout', 'search'],
  'kind-b': ['default', 'kube-system', 'staging'],
} as const

export type FixtureContext = keyof typeof NAMESPACES

const APPS = [
  'api',
  'web',
  'worker',
  'cache',
  'ingest',
  'auth',
  'billing',
  'search',
  'queue',
  'cron',
]

/** mulberry32: tiny seeded PRNG. */
function rng(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

function hash(i: number, salt: number): string {
  return ((i * 2654435761 + salt) >>> 0).toString(36).padStart(5, '0').slice(-5)
}

/** `count` pods spread across the context's namespaces, sorted by name like the backend. */
export function genPods(
  count: number,
  context: FixtureContext = 'kind-a',
  seed = 42,
): PodSummary[] {
  const rand = rng(seed)
  const namespaces = NAMESPACES[context]
  const pods: PodSummary[] = []
  for (let i = 0; i < count; i++) {
    const app = APPS[i % APPS.length] ?? 'app'
    const namespace = namespaces[i % namespaces.length] ?? 'default'
    const r = rand()
    const phase = r < 0.9 ? 'Running' : r < 0.95 ? 'Pending' : r < 0.98 ? 'Succeeded' : 'Failed'
    const total = 1 + (i % 3)
    const ready = phase === 'Running' ? (r < 0.85 ? total : total - 1) : 0
    pods.push({
      name: `${app}-${hash(i, seed)}-${hash(i, seed + 1)}`,
      namespace,
      phase,
      node: `${context}-worker-${(i % 8) + 1}`,
      ready,
      total,
      restarts: r < 0.8 ? 0 : Math.floor(rand() * 40),
    })
  }
  return pods.sort((a, b) => a.name.localeCompare(b.name))
}

export function genDeployments(context: FixtureContext = 'kind-a'): WorkloadSummary[] {
  const out: WorkloadSummary[] = []
  for (const namespace of NAMESPACES[context]) {
    APPS.slice(0, context === 'kind-a' ? 6 : 3).forEach((app, i) => {
      const desired = 1 + (i % 4)
      const ready = app === 'billing' ? desired - 1 : desired
      out.push({ name: app, namespace, desired, ready, updated: desired, available: ready })
    })
  }
  return out.sort((a, b) => a.name.localeCompare(b.name) || a.namespace.localeCompare(b.namespace))
}

/** A full `PodDetails` read for `pod`, including `raw_json` (the source of the YAML view). */
export function podDetails(pod: Pick<PodSummary, 'namespace' | 'name' | 'node'>): PodDetails {
  const app = pod.name.split('-')[0] ?? 'app'
  const labels = { app, 'pod-template-hash': '7f9c6d5b4' }
  const manifest = {
    apiVersion: 'v1',
    kind: 'Pod',
    metadata: {
      name: pod.name,
      namespace: pod.namespace,
      labels,
      creationTimestamp: '2026-09-20T08:15:00Z',
      ownerReferences: [{ kind: 'ReplicaSet', name: `${app}-7f9c6d5b4`, controller: true }],
    },
    spec: {
      nodeName: pod.node,
      containers: [
        {
          name: app,
          image: `registry.example.com/${app}:1.4.2`,
          ports: [{ name: 'http', containerPort: 8080, protocol: 'TCP' }],
          resources: {
            requests: { cpu: '100m', memory: '128Mi' },
            limits: { cpu: '500m', memory: '256Mi' },
          },
        },
      ],
    },
    status: { phase: 'Running' },
  }
  return {
    namespace: pod.namespace,
    name: pod.name,
    creation_timestamp: '2026-09-20T08:15:00Z',
    labels,
    annotations: { 'kubectl.kubernetes.io/restartedAt': '2026-09-20T08:14:00Z' },
    containers: [app],
    ports: [{ container: app, name: 'http', port: 8080, protocol: 'TCP' }],
    resource_specs: [
      {
        container: app,
        cpu_request_millicores: 100,
        cpu_limit_millicores: 500,
        memory_request_bytes: 134217728,
        memory_limit_bytes: 268435456,
      },
    ],
    owner: { kind: 'ReplicaSet', name: `${app}-7f9c6d5b4` },
    resolved_owner: { kind: 'Deployment', name: app },
    persistent_volume_claims: [],
    raw_json: JSON.stringify(manifest, null, 2),
  }
}

/** Cluster events about `pod` (the inspector's Events tab filters by involved object). */
export function podEvents(pod: Pick<PodSummary, 'namespace' | 'name' | 'node'>): KubernetesEvent[] {
  const base = {
    namespace: pod.namespace,
    involved_object_kind: 'Pod',
    involved_object_name: pod.name,
  }
  return [
    {
      ...base,
      name: `${pod.name}.1`,
      event_type: 'Normal',
      reason: 'Scheduled',
      message: `Successfully assigned ${pod.namespace}/${pod.name} to ${pod.node}`,
      count: 1,
      first_timestamp: '2026-09-20T08:15:00Z',
      last_timestamp: '2026-09-20T08:15:00Z',
      source: 'default-scheduler',
    },
    {
      ...base,
      name: `${pod.name}.2`,
      event_type: 'Normal',
      reason: 'Pulled',
      message: 'Container image "registry.example.com/api:1.4.2" already present on machine',
      count: 1,
      first_timestamp: '2026-09-20T08:15:02Z',
      last_timestamp: '2026-09-20T08:15:02Z',
      source: 'kubelet',
    },
    {
      ...base,
      name: `${pod.name}.3`,
      event_type: 'Warning',
      reason: 'BackOff',
      message: 'Back-off restarting failed container api in pod',
      count: 4,
      first_timestamp: '2026-09-20T08:20:11Z',
      last_timestamp: '2026-09-20T08:31:47Z',
      source: 'kubelet',
    },
  ]
}
