/**
 * Deterministic generators for large, realistic lists (pods-500, pods-10k) and per-object
 * reads. Seeded, so the same fixture is byte-identical on every run.
 */
import type {
  ContainerLifecycle,
  KubernetesEvent,
  PodDetails,
  PodSummary,
  WorkloadDetails,
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
    containers_lifecycle: [
      {
        name: app,
        is_init: false,
        is_ephemeral: false,
        image: `registry.example.com/${app}:1.4.2`,
        ready: true,
        state: {
          phase: 'running',
          reason: null,
          exit_code: null,
          started_at: null,
          finished_at: null,
        },
        restart_count: 0,
        last_state: null,
        probes: [],
        hooks: [],
      },
    ],
  }
}

/**
 * 008-pod-lifecycle-diagnostics (T011): a `PodDetails` fixture with a richer
 * `containers_lifecycle` — two init containers (one succeeded, one failed), and an app container
 * with a full set of probes, a `postStart`/`preStop` hook pair, and a prior `OOMKilled` restart —
 * for the five `us{1..5}-pod-lifecycle-*` e2e specs. Independent of `podDetails()` so its existing
 * callers are unaffected.
 */
export function podDetailsWithLifecycle(
  pod: Pick<PodSummary, 'namespace' | 'name' | 'node'>,
): PodDetails {
  const base = podDetails(pod)
  const app = base.containers[0] ?? 'app'
  const containersLifecycle: ContainerLifecycle[] = [
    {
      name: 'init-migrate',
      is_init: true,
      is_ephemeral: false,
      image: 'registry.example.com/migrate:1.0.0',
      ready: false,
      state: {
        phase: 'terminated',
        reason: 'Completed',
        exit_code: 0,
        started_at: '2026-09-20T08:14:50Z',
        finished_at: '2026-09-20T08:14:55Z',
      },
      restart_count: 0,
      last_state: null,
      probes: [],
      hooks: [],
    },
    {
      name: 'init-verify',
      is_init: true,
      is_ephemeral: false,
      image: 'registry.example.com/verify:1.0.0',
      ready: false,
      state: {
        phase: 'terminated',
        reason: 'Error',
        exit_code: 1,
        started_at: '2026-09-20T08:14:56Z',
        finished_at: '2026-09-20T08:14:58Z',
      },
      restart_count: 0,
      last_state: null,
      probes: [],
      hooks: [],
    },
    {
      name: app,
      is_init: false,
      is_ephemeral: false,
      image: `registry.example.com/${app}:1.4.2`,
      ready: false,
      state: {
        phase: 'running',
        reason: null,
        exit_code: null,
        started_at: '2026-09-20T08:15:05Z',
        finished_at: null,
      },
      restart_count: 3,
      last_state: {
        phase: 'terminated',
        reason: 'OOMKilled',
        exit_code: 137,
        started_at: '2026-09-20T08:10:00Z',
        finished_at: '2026-09-20T08:12:30Z',
      },
      probes: [
        {
          probe_type: 'readiness',
          mechanism: 'httpGet',
          target: 'GET /healthz:8080',
          period_seconds: 10,
          failure_threshold: 3,
          success_threshold: 1,
        },
        {
          probe_type: 'liveness',
          mechanism: 'httpGet',
          target: 'GET /livez:8080',
          period_seconds: 15,
          failure_threshold: 3,
          success_threshold: 1,
        },
        {
          probe_type: 'startup',
          mechanism: 'tcpSocket',
          target: 'tcp:8080',
          period_seconds: 5,
          failure_threshold: 30,
          success_threshold: 1,
        },
      ],
      hooks: [
        { hook_type: 'postStart', mechanism: 'exec', target: 'exec: /bin/sh -c warm-cache.sh' },
        { hook_type: 'preStop', mechanism: 'httpGet', target: 'GET /shutdown:8080' },
      ],
    },
    {
      name: 'sidecar',
      is_init: false,
      is_ephemeral: false,
      image: 'registry.example.com/sidecar:1.0.0',
      ready: true,
      state: {
        phase: 'running',
        reason: null,
        exit_code: null,
        started_at: '2026-09-20T08:15:05Z',
        finished_at: null,
      },
      restart_count: 0,
      last_state: null,
      probes: [],
      hooks: [],
    },
  ]
  return {
    ...base,
    containers: [...base.containers, 'sidecar'],
    containers_lifecycle: containersLifecycle,
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

/**
 * 008-pod-lifecycle-diagnostics (T011): events correlating with `podDetailsWithLifecycle()`'s
 * probe/hook failures, plus an event with an unrecognized `reason` (FR-019 — still shown as-is).
 */
export function podEventsWithLifecycle(
  pod: Pick<PodSummary, 'namespace' | 'name' | 'node'>,
): KubernetesEvent[] {
  const app = pod.name.split('-')[0] ?? 'app'
  const base = {
    namespace: pod.namespace,
    involved_object_kind: 'Pod' as const,
    involved_object_name: pod.name,
  }
  return [
    ...podEvents(pod),
    {
      ...base,
      name: `${pod.name}.unhealthy`,
      event_type: 'Warning',
      reason: 'Unhealthy',
      message: `Liveness probe failed: HTTP probe failed with statuscode: 503 (container ${app})`,
      count: 2,
      first_timestamp: '2026-09-20T08:15:10Z',
      last_timestamp: '2026-09-20T08:15:40Z',
      source: 'kubelet',
    },
    {
      ...base,
      name: `${pod.name}.poststart-failed`,
      event_type: 'Warning',
      reason: 'FailedPostStartHook',
      message: `Exec lifecycle hook ([/bin/sh -c warm-cache.sh]) for Container "${app}" in Pod "${pod.name}_${pod.namespace}(uid)" failed`,
      count: 1,
      first_timestamp: '2026-09-20T08:15:05Z',
      last_timestamp: '2026-09-20T08:15:05Z',
      source: 'kubelet',
    },
    {
      ...base,
      name: `${pod.name}.unknown`,
      event_type: 'Normal',
      reason: 'SomeUnknownReason',
      message: 'An event reason Fathom does not specifically recognize.',
      count: 1,
      first_timestamp: '2026-09-20T08:16:00Z',
      last_timestamp: '2026-09-20T08:16:00Z',
      source: 'kubelet',
    },
  ]
}

/** A full `WorkloadDetails` read for deployment `d`, with its pods from `pods`. */
export function deploymentDetails(
  d: WorkloadSummary,
  pods: readonly PodSummary[],
): WorkloadDetails {
  const labels = { app: d.name }
  const manifest = {
    apiVersion: 'apps/v1',
    kind: 'Deployment',
    metadata: {
      name: d.name,
      namespace: d.namespace,
      labels,
      creationTimestamp: '2026-09-18T11:02:00Z',
      annotations: { 'deployment.kubernetes.io/revision': '3' },
    },
    spec: {
      replicas: d.desired,
      selector: { matchLabels: labels },
      template: {
        metadata: { labels },
        spec: {
          containers: [
            {
              name: d.name,
              image: `registry.example.com/${d.name}:1.4.2`,
              ports: [{ name: 'http', containerPort: 8080, protocol: 'TCP' }],
            },
          ],
        },
      },
    },
    status: {
      replicas: d.desired,
      readyReplicas: d.ready,
      updatedReplicas: d.updated,
      availableReplicas: d.available,
    },
  }
  return {
    namespace: d.namespace,
    name: d.name,
    creation_timestamp: '2026-09-18T11:02:00Z',
    labels,
    annotations: { 'deployment.kubernetes.io/revision': '3' },
    containers: [d.name],
    ports: [{ container: d.name, name: 'http', port: 8080, protocol: 'TCP' }],
    pod_selector: `app=${d.name}`,
    pods: pods
      .filter((p) => p.namespace === d.namespace && p.name.startsWith(`${d.name}-`))
      .slice(0, d.desired),
    raw_json: JSON.stringify(manifest, null, 2),
  }
}
