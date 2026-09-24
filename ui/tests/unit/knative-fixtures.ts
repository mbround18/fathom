import type { KnativeOverview, KnativeRevisionSummary } from '@/protocol/types'

export function revision(over: Partial<KnativeRevisionSummary> = {}): KnativeRevisionSummary {
  return {
    name: 'hello-00001',
    namespace: 'default',
    service_name: 'hello',
    ready: 'Ready',
    actual_replicas: 1,
    desired_replicas: 1,
    scale_reason: { kind: 'AtDesiredScale' },
    traffic_percent: 80,
    ...over,
  }
}

export function overview(over: Partial<KnativeOverview> = {}): KnativeOverview {
  return {
    installed: true,
    services: [
      {
        name: 'hello',
        namespace: 'default',
        ready: 'Ready',
        latest_ready_revision: 'hello-00002',
        latest_created_revision: 'hello-00002',
      },
    ],
    revisions: [revision(), revision({ name: 'hello-00002', traffic_percent: 20 })],
    routes: [
      {
        name: 'hello',
        namespace: 'default',
        service_name: 'hello',
        ready: 'Ready',
        url: 'http://hello.default.example.com',
        traffic: [
          { revision_name: 'hello-00001', percent: 80, tag: null, revision_missing: false },
          { revision_name: 'hello-00002', percent: 20, tag: null, revision_missing: false },
        ],
      },
    ],
    ...over,
  }
}
