/**
 * 048 T032: the container-runtime slice and the presence gate in front of the sidebar.
 *
 * Two things are worth pinning here. Staleness: a runtime that goes away mid-scene must leave the
 * last inventory on screen with a reason beside it, never blank it (FR-003). And visibility: the
 * group is absent until a probe has answered, and absent again if the answer is "no runtime" —
 * the same rendering for two different facts, which is exactly why `probed` exists.
 */
import { beforeEach, describe, expect, it } from 'vitest'

import { runtimeVisible } from '@/resources/nav'
import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'
import { ingest } from '@/store/ingest'

import type { BackendCmd } from '@/protocol/types'

import {
  ALL_CONTAINERS,
  adoptionUpdated,
  clusterLinksUpdated,
  inventory,
  inventoryUnavailable,
  project,
  projectGroup,
  inventoryUpdated,
  operationFinished,
  operationPreviewed,
  operationRefused,
  presenceUpdated,
  usageSampled,
} from '../fixtures/containers'

const rt = () => useStore.getState().containerRuntime

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
})

describe('container runtime slice', () => {
  it('reads nothing before a probe answers', () => {
    expect(rt().probed).toBe(false)
    expect(rt().presence).toBeUndefined()
    expect(rt().inventory).toBeUndefined()
  })

  it('records presence and marks the probe answered, including when there is no runtime', () => {
    ingest(presenceUpdated('absent'))
    expect(rt().probed).toBe(true)
    expect(rt().presence?.state).toBe('absent')
    expect(rt().presence?.endpoint).toBeNull()
  })

  it('keeps the runtime’s own words for an unreachable socket', () => {
    ingest(presenceUpdated('unreachable'))
    expect(rt().presence?.detail).toContain('permission denied')
    expect(rt().presence?.endpoint).toBe('unix:///var/run/docker.sock')
  })

  it('marks the inventory stale without blanking it, and un-stales only on new facts', () => {
    const inv = inventory({
      projects: [project({ group: projectGroup('shop'), containers: ALL_CONTAINERS })],
    })
    ingest(inventoryUpdated(inv))
    ingest(
      inventoryUnavailable('Cannot connect to the Docker daemon at unix:///var/run/docker.sock'),
    )
    expect(rt().stale).toContain('Cannot connect')
    expect(rt().inventory?.projects[0]?.containers).toHaveLength(ALL_CONTAINERS.length)

    ingest(inventoryUpdated(inv))
    expect(rt().stale).toBeUndefined()
  })

  it('records adoption and the fallback that explains a refusal', () => {
    ingest(adoptionUpdated(['shop'], true))
    expect(rt().adopted).toEqual(['shop'])
    expect(rt().adoptionFellBack).toBe(true)
  })

  it('records the cluster links both directions share', () => {
    ingest(
      clusterLinksUpdated([
        { cluster: 'fathom-dev', container_ids: ['a'], managed_by_fathom: true },
      ]),
    )
    expect(rt().clusterLinks[0]?.cluster).toBe('fathom-dev')
  })

  it('counts a tick that measured nothing as a tick, and not as figures of zero', () => {
    ingest(usageSampled([]))
    expect(rt().usage.ticks).toBe(1)
    expect(rt().usage.latest).toEqual({})
  })
})

/**
 * 048 T047: the two-phase mutation path, as the slice sees it.
 *
 * The things worth pinning here are all refusals to guess: nothing moves to `finished` without the
 * backend's own outcome, a reply for a dismissed operation is dropped rather than resurrecting it,
 * and the argv the operator approved is the one `IssueContainerOperation` names by `preview_id` —
 * this side never sends a command.
 */
describe('the two-phase mutation path (US2)', () => {
  const STOP_SHOP: Parameters<
    ReturnType<typeof useStore.getState>['previewContainerOperation']
  >[0] = { kind: 'lifecycle', action: 'stop', scope: { kind: 'project', project: 'shop' } }

  const capture = () => {
    const sent: BackendCmd[] = []
    useStore.setState({ send: (c) => void sent.push(c) })
    return sent
  }

  const previewOf = (over: Partial<ReturnType<typeof basePreview>> = {}) => ({
    ...basePreview(),
    ...over,
  })

  function basePreview() {
    return {
      preview_id: 'pv-1',
      request: STOP_SHOP,
      command: ['docker', 'stop', 'shop-web-1', 'shop-db-1'],
      destructive: false,
      removals: [],
      total_size_bytes: null,
      blast_radius: [],
      excluded: [],
      breaks_clusters: [],
      required_confirmation: null,
      as_of: '2026-10-02T12:00:00Z',
    }
  }

  it('asks for a preview and runs nothing', () => {
    const sent = capture()
    useStore.getState().previewContainerOperation(STOP_SHOP)
    expect(sent).toHaveLength(1)
    expect(sent[0]?.type).toBe('PreviewContainerOperation')
    expect(rt().operation?.phase).toBe('previewing')
    expect(rt().operation?.preview).toBeUndefined()
  })

  it('issues by preview id, never by command: the argv stays the backend’s', () => {
    capture()
    useStore.getState().previewContainerOperation(STOP_SHOP)
    const requestId = rt().operation?.requestId ?? ''
    ingest(operationPreviewed(requestId, previewOf()))
    expect(rt().operation?.phase).toBe('previewed')
    expect(rt().operation?.preview?.command).toEqual(['docker', 'stop', 'shop-web-1', 'shop-db-1'])

    const sent = capture()
    useStore.getState().issueContainerOperation(null)
    expect(sent).toEqual([
      {
        type: 'IssueContainerOperation',
        data: { request_id: requestId, preview_id: 'pv-1', typed_confirmation: null },
      },
    ])
    // FR-021: issued, not finished. Nothing has changed until the runtime says it did.
    expect(rt().operation?.phase).toBe('issued')
    expect(rt().operation?.outcome).toBeUndefined()
  })

  it('will not issue without a preview', () => {
    const sent = capture()
    useStore.getState().previewContainerOperation(STOP_SHOP)
    sent.length = 0
    useStore.getState().issueContainerOperation(null)
    expect(sent).toEqual([])
  })

  it('keeps every per-member outcome, including the one that failed', () => {
    capture()
    useStore.getState().previewContainerOperation(STOP_SHOP)
    const requestId = rt().operation?.requestId ?? ''
    ingest(operationPreviewed(requestId, previewOf()))
    useStore.getState().issueContainerOperation(null)
    ingest(
      operationFinished(requestId, {
        preview_id: 'pv-1',
        command: ['docker', 'stop', 'shop-web-1', 'shop-db-1'],
        overall: 'failed',
        objects: [
          {
            object: { class: 'container', id: 'web', name: 'shop-web-1' },
            status: 'succeeded',
            error: null,
          },
          {
            object: { class: 'container', id: 'db', name: 'shop-db-1' },
            status: 'failed',
            error: 'Error response from daemon: No such container: shop-db-1',
          },
        ],
        discrepancies: [],
      }),
    )
    const outcome = rt().operation?.outcome
    expect(rt().operation?.phase).toBe('finished')
    expect(outcome?.overall).toBe('failed')
    expect(outcome?.objects).toHaveLength(2)
    // FR-022: the runtime's own sentence, not a rewording of it.
    expect(outcome?.objects[1]?.error).toBe(
      'Error response from daemon: No such container: shop-db-1',
    )
  })

  it('keeps a refusal where the control that asked can read it', () => {
    capture()
    useStore.getState().previewContainerOperation(STOP_SHOP)
    const requestId = rt().operation?.requestId ?? ''
    ingest(operationRefused(requestId, { kind: 'not_adopted', detail: 'shop' }))
    expect(rt().operation?.phase).toBe('refused')
    expect(rt().operation?.refusal).toEqual({ kind: 'not_adopted', detail: 'shop' })
  })

  it('drops a reply for an operation the operator dismissed', () => {
    capture()
    useStore.getState().previewContainerOperation(STOP_SHOP)
    const requestId = rt().operation?.requestId ?? ''
    useStore.getState().dismissContainerOperation()
    expect(rt().operation).toBeUndefined()
    ingest(operationPreviewed(requestId, previewOf()))
    expect(rt().operation).toBeUndefined()
  })

  it('adopts and releases by name, and nothing else', () => {
    const sent = capture()
    useStore.getState().adoptContainerProject('shop')
    useStore.getState().releaseContainerProject('shop')
    expect(sent).toEqual([
      { type: 'AdoptContainerProject', data: { project: 'shop' } },
      { type: 'ReleaseContainerProject', data: { project: 'shop' } },
    ])
  })
})

describe('sidebar presence gate (FR-002)', () => {
  const ctx = (over: Partial<Parameters<typeof runtimeVisible>[0]> = {}) => ({
    hosted: false,
    desktop: true,
    installed: () => undefined,
    ...over,
  })

  it('hides the group until the probe has answered', () => {
    expect(runtimeVisible(ctx())).toBe(false)
  })

  it('hides the group when there is no runtime on this computer', () => {
    expect(runtimeVisible(ctx({ runtime: 'absent' }))).toBe(false)
  })

  it('shows the group for a runtime that is installed but stopped', () => {
    expect(runtimeVisible(ctx({ runtime: 'stopped' }))).toBe(true)
    expect(runtimeVisible(ctx({ runtime: 'unreachable' }))).toBe(true)
    expect(runtimeVisible(ctx({ runtime: 'ready' }))).toBe(true)
  })

  it('never shows the group in hosted mode: a server pod has no local daemon', () => {
    expect(runtimeVisible(ctx({ hosted: true, desktop: false, runtime: 'ready' }))).toBe(false)
  })
})
