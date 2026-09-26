/**
 * Contract test (T032, contracts/ipc.md § Verification): every fixture is a valid `EventBatch`
 * per the generated bindings (checked by `tsc` through `satisfies`), is plain JSON, and speaks
 * protocol version 1.
 */
import { describe, expect, it } from 'vitest'

import type { BackendCmd, EventBatch, SystemEvent } from '@/protocol/types'

import {
  CONTEXTS,
  FIXTURES,
  PODS_500,
  SCENARIOS,
  STALE_POD_NAME,
  type FixtureName,
  type ScenarioName,
} from '../fixtures'
import type { FakeBackendState, Reply } from '../fixtures/scenarios'

// Compile-time contract: drift in the generated types breaks this line under `pnpm typecheck`.
const fixtures = FIXTURES satisfies Record<FixtureName, EventBatch>
const entries = Object.entries(fixtures) as [FixtureName, EventBatch][]

function systemEvents(batch: EventBatch): SystemEvent[] {
  return batch.envelopes.flatMap((e) => (e.event.type === 'System' ? [e.event.data] : []))
}

describe('fixtures match the generated protocol', () => {
  it.each(entries)('%s: every envelope is version 1', (_name, batch) => {
    expect(batch.dropped).toBeGreaterThanOrEqual(0)
    for (const env of batch.envelopes) expect(env.version).toBe(1)
  })

  it.each(entries)('%s: survives a JSON round trip unchanged (wire-shaped)', (_name, batch) => {
    expect(JSON.parse(JSON.stringify(batch))).toEqual(batch)
  })

  it.each(entries)('%s: seq is monotonic within the batch', (_name, batch) => {
    const seqs = batch.envelopes.map((e) => e.seq)
    expect(seqs).toEqual([...seqs].sort((a, b) => a - b))
  })

  it('two-contexts lists kind-a and kind-b with kind-a current', () => {
    const updated = systemEvents(fixtures['two-contexts']).find((e) => e.type === 'ContextsUpdated')
    expect(updated?.data).toEqual({ contexts: [...CONTEXTS], current: 'kind-a' })
  })

  it('no-kubeconfig reports no contexts and an error notification', () => {
    const events = systemEvents(fixtures['no-kubeconfig'])
    expect(events).toContainEqual({
      type: 'ContextsUpdated',
      data: { contexts: [], current: null },
    })
    expect(events.some((e) => e.type === 'Notification' && e.data.level === 'Error')).toBe(true)
  })

  it('pods-10k holds 10,000 uniquely named pods', () => {
    const env = fixtures['pods-10k'].envelopes[0]
    if (env?.event.type !== 'ResourceList' || env.event.data.type !== 'PodsUpdated') {
      throw new Error('pods-10k must be one PodsUpdated envelope')
    }
    const names = new Set(env.event.data.data.map((p) => `${p.namespace}/${p.name}`))
    expect(names.size).toBe(10_000)
  })

  it('stale-kind-a is routed to kind-a and not part of the normal pod list', () => {
    const env = fixtures['stale-kind-a'].envelopes[0]
    expect(env?.route.context).toBe('kind-a')
    expect(PODS_500.some((p) => p.name === STALE_POD_NAME)).toBe(false)
  })

  it('pod-detail carries raw_json that parses (the YAML view source)', () => {
    const env = fixtures['pod-detail'].envelopes[0]
    if (env?.event.type !== 'ResourceRead' || env.event.data.type !== 'PodDetailsUpdated') {
      throw new Error('pod-detail must be one PodDetailsUpdated envelope')
    }
    expect(JSON.parse(env.event.data.data.raw_json)).toMatchObject({ kind: 'Pod' })
  })
})

describe('scenario replies are valid batches', () => {
  const cmds: BackendCmd[] = [
    { type: 'LoadUiConfig' },
    { type: 'RefreshContexts' },
    { type: 'Initialize' },
    { type: 'RefreshNamespaces' },
    { type: 'RefreshPods' },
    { type: 'RefreshDeployments' },
    { type: 'SetNamespaceFilter', data: { namespaces: ['payments'] } },
    {
      type: 'SelectPod',
      data: { namespace: PODS_500[0]?.namespace ?? '', name: PODS_500[0]?.name ?? '' },
    },
    { type: 'RefreshEvents' },
    {
      type: 'FetchManifests',
      data: {
        request_id: 'r1',
        objects: [
          { api_version: 'apps/v1', kind: 'Deployment', namespace: 'default', name: 'api' },
          { api_version: 'apps/v1', kind: 'Deployment', namespace: 'default', name: 'missing' },
        ],
      },
    },
    { type: 'PreviewKubeconfigExport', data: { context: 'kind-a' } },
    {
      type: 'ExportKubeconfig',
      data: {
        context: 'kind-b',
        path: '/tmp/k.yaml',
        include_embedded_credentials: false,
        overwrite: false,
      },
    },
    { type: 'SetContext', data: { context: 'kind-b' } },
  ]
  const names = Object.keys(SCENARIOS) as ScenarioName[]

  it.each(names)('%s', (name) => {
    const scenario = SCENARIOS[name]
    const state: FakeBackendState = structuredClone(scenario.initialState)
    const replies = scenario.replies as Partial<Record<BackendCmd['type'], Reply>>
    for (const batch of scenario.onSubscribe) {
      for (const env of batch.envelopes) expect(env.version).toBe(1)
    }
    for (const cmd of cmds) {
      for (const batch of replies[cmd.type]?.(cmd, state) ?? []) {
        for (const env of batch.envelopes) expect(env.version).toBe(1)
        expect(JSON.parse(JSON.stringify(batch))).toEqual(batch)
      }
    }
  })
})
