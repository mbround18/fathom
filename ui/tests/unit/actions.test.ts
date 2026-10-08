/**
 * US3 action flow (T093/T094): descriptor plans and `runAction` — pending by request id,
 * resolution by `CommandFinished`, verbatim errors, no optimistic change, and the timeout.
 */
import { toast } from 'sonner'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { BackendCmd, PodSummary, WorkloadSummary } from '@/protocol/types'
// `@/store` must be imported before any descriptor module that (transitively, via
// `resources/related.tsx`) imports `@/store` itself — otherwise the reentrant cycle leaves
// that descriptor's module mid-evaluation and `ALL_DESCRIPTORS` picks up `undefined` for it.
// The app's own entry point (`main.tsx`) naturally imports `@/store` first; this just matches
// that order.
import { initialData, useStore } from '@/store'
import { deployments } from '@/resources/deployments'
import { actionsFor } from '@/resources/actions'
import { pods } from '@/resources/pods'
import {
  commandFinished,
  resetActions,
  runAction,
  setActionTimeout,
  summarize,
} from '@/store/actions'

import { PODS_500 } from '../fixtures'
import { genDeployments } from '../fixtures/gen-pods'

vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), loading: vi.fn() } }))

const [P1, P2, P3] = PODS_500 as [PodSummary, PodSummary, PodSummary, ...PodSummary[]]
const DEPS = genDeployments('kind-a')
const [D1, D2] = DEPS as [WorkloadSummary, WorkloadSummary, ...WorkloadSummary[]]
function must<T>(v: T | undefined, what: string): T {
  if (v === undefined) throw new Error(`missing ${what}`)
  return v
}
const podDelete = must(
  pods.actions.find((a) => a.id === 'delete'),
  'pod delete',
)
const depAction = (id: string) =>
  must(
    deployments.actions.find((a) => a.id === id),
    id,
  )

let sent: BackendCmd[]
let nextId: number
let refresh: ReturnType<typeof vi.fn<(section: string) => void>>

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  setActionTimeout()
  vi.clearAllMocks()
  sent = []
  nextId = 100
  refresh = vi.fn<(section: string) => void>()
  useStore.setState({
    request: (cmd) => {
      sent.push(cmd)
      return Promise.resolve(nextId++)
    },
    refreshSection: refresh,
  })
})

afterEach(() => {
  vi.useRealTimers()
})

/** Wait until `n` requests are pending, then return their ids. */
async function pendingIds(n: number): Promise<number[]> {
  await vi.waitFor(() => expect(Object.keys(useStore.getState().pending)).toHaveLength(n))
  return Object.keys(useStore.getState().pending).map(Number)
}

describe('descriptor actions (T094)', () => {
  it('pods: delete + debug; one row → DeletePod, several → one BulkDelete naming each', () => {
    expect(pods.actions.map((a) => a.id)).toEqual(['delete', 'debug'])
    expect(podDelete.plan([P1], {})).toEqual([
      { cmd: { type: 'DeletePod', data: { namespace: P1.namespace, name: P1.name } }, rows: [P1] },
    ])
    const bulk = podDelete.plan([P1, P2, P3], {})
    expect(bulk).toHaveLength(1)
    expect(bulk[0]?.cmd).toEqual({
      type: 'BulkDelete',
      data: { items: [P1, P2, P3].map((p) => [p.namespace, p.name]) },
    })
  })

  it('deployments: scale, restart, delete; bulk sends one command per row', () => {
    expect(deployments.actions.map((a) => a.id)).toEqual(['scale', 'restart', 'delete'])
    expect(depAction('scale').plan([D1], { replicas: 3 })[0]?.cmd).toEqual({
      type: 'ScaleDeployment',
      data: { namespace: D1.namespace, name: D1.name, replicas: 3 },
    })
    expect(depAction('scale').current?.(D1)).toBe(D1.desired)
    expect(depAction('scale').plan([D1], {})).toEqual([])
    expect(depAction('scale').plan([D1], { replicas: -1 })).toEqual([])
    expect(
      depAction('restart')
        .plan([D1, D2], {})
        .map((s) => s.cmd.type),
    ).toEqual(['RestartDeployment', 'RestartDeployment'])
    expect(
      depAction('delete')
        .plan([D1, D2], {})
        .map((s) => s.cmd),
    ).toEqual([
      { type: 'DeleteDeployment', data: { namespace: D1.namespace, name: D1.name } },
      { type: 'DeleteDeployment', data: { namespace: D2.namespace, name: D2.name } },
    ])
  })

  it('actionsFor: none for 0 rows, all for 1, bulk-capable only for several', () => {
    const two = genDeployments().slice(0, 2)
    expect(actionsFor(deployments.actions, [])).toEqual([])
    expect(actionsFor(deployments.actions, two.slice(0, 1)).map((x) => x.id)).toEqual([
      'scale',
      'restart',
      'delete',
    ])
    expect(actionsFor(deployments.actions, two).map((x) => x.id)).toEqual(['restart', 'delete'])
  })

  it('summarize names one object by namespace/name', () => {
    expect(summarize(pods, [P1])).toBe(`pod ${P1.namespace}/${P1.name}`)
    expect(summarize(pods, [P1, P2])).toBe('2 pods')
  })
})

describe('runAction (T093)', () => {
  it('success: pending until CommandFinished, then ✓ toast, selection cleared, list refreshed', async () => {
    useStore.setState({ selection: { 'list-1': [pods.rowKey(P1), pods.rowKey(P2)] } })
    const run = runAction(pods, podDelete, [P1, P2], { paneId: 'list-1' })
    const id = must((await pendingIds(1))[0], 'request id')
    expect(sent).toEqual([
      {
        type: 'BulkDelete',
        data: {
          items: [
            [P1.namespace, P1.name],
            [P2.namespace, P2.name],
          ],
        },
      },
    ])
    expect(useStore.getState().pending[id]?.label).toBe('Delete 2 pods')
    // Nothing optimistic: the selection and lists stay until the result arrives.
    expect(useStore.getState().selection['list-1']).toHaveLength(2)
    expect(refresh).not.toHaveBeenCalled()

    commandFinished(id, { ok: true, error: null })
    const outcome = await run
    expect(outcome.ok).toEqual([P1, P2])
    expect(useStore.getState().pending).toEqual({})
    expect(useStore.getState().selection['list-1']).toEqual([])
    expect(refresh).toHaveBeenCalledWith(pods.section)
    expect(toast.success).toHaveBeenCalledWith(
      '✓ Deleted 2 pods',
      expect.objectContaining({ testId: 'action-toast' }),
    )
  })

  it('failure: ✗ toast with the verbatim backend error; selection and list untouched', async () => {
    const error = 'pods "api-0" is forbidden: User "dev" cannot delete resource "pods"'
    useStore.setState({ selection: { 'list-1': [pods.rowKey(P1)] } })
    const run = runAction(pods, podDelete, [P1], { paneId: 'list-1' })
    const id = must((await pendingIds(1))[0], 'request id')
    commandFinished(id, { ok: false, error })
    const outcome = await run
    expect(outcome.failed).toEqual([{ row: P1, error }])
    expect(toast.error).toHaveBeenCalledWith(
      `✖ Delete failed: pod ${P1.namespace}/${P1.name}`,
      expect.objectContaining({ description: error, duration: Infinity }),
    )
    expect(useStore.getState().selection['list-1']).toEqual([pods.rowKey(P1)])
    expect(refresh).not.toHaveBeenCalled()
  })

  it('a result that beats the send reply is not lost', async () => {
    useStore.setState({
      request: (cmd) => {
        sent.push(cmd)
        commandFinished(7, { ok: true, error: null })
        return Promise.resolve(7)
      },
    })
    const outcome = await runAction(pods, podDelete, [P1])
    expect(outcome.ok).toEqual([P1])
  })

  it('a send that rejects is reported verbatim and nothing stays pending', async () => {
    useStore.setState({ request: () => Promise.reject(new Error('backend gone')) })
    const outcome = await runAction(pods, podDelete, [P1])
    expect(outcome.failed[0]?.error).toBe('backend gone')
    expect(useStore.getState().pending).toEqual({})
  })

  it('per-row commands: partial failure keeps only the failed rows selected', async () => {
    const keys = [deployments.rowKey(D1), deployments.rowKey(D2)]
    useStore.setState({ selection: { 'list-2': keys } })
    const run = runAction(deployments, depAction('delete'), [D1, D2], { paneId: 'list-2' })
    const [a, b] = (await pendingIds(2)).sort((x, y) => x - y)
    commandFinished(must(a, 'a'), { ok: true, error: null })
    commandFinished(must(b, 'b'), { ok: false, error: 'conflict' })
    const outcome = await run
    expect(outcome.ok).toEqual([D1])
    expect(useStore.getState().selection['list-2']).toEqual([keys[1]])
    expect(refresh).toHaveBeenCalledWith(deployments.section)
    expect(toast.error).toHaveBeenCalledWith(
      '✖ Delete: 1 of 2 failed',
      expect.objectContaining({
        description: `${D2.namespace}/${D2.name}: conflict\n1 succeeded.`,
      }),
    )
  })

  it('no result within the timeout: "no response" toast, pending cleared, late result reported', async () => {
    setActionTimeout(20)
    const id = nextId
    const outcome = await runAction(deployments, depAction('scale'), [D1], {
      input: { replicas: 3 },
    })
    expect(outcome.timedOut).toEqual([D1])
    expect(useStore.getState().pending).toEqual({})
    expect(toast.error).toHaveBeenCalledWith(
      `✖ No response from backend: Scale deployment ${D1.namespace}/${D1.name} to 3`,
      expect.anything(),
    )
    commandFinished(id, { ok: true, error: null })
    expect(toast.success).toHaveBeenCalledWith(
      `✓ Scaled ${D1.namespace}/${D1.name} (late result)`,
      expect.anything(),
    )
    expect(refresh).toHaveBeenCalledWith(deployments.section)
  })

  it('an invalid scale input sends nothing', async () => {
    const outcome = await runAction(deployments, depAction('scale'), [D1], { input: {} })
    expect(sent).toEqual([])
    expect(outcome.ok).toEqual([])
    expect(toast.loading).not.toHaveBeenCalled()
  })
})
