/**
 * The Review/Confirm/Decline adapter (013-agent-sdk-ops, US3/P1 — Phase 5), constitution
 * Principle I: this is the single most important test file in the feature. It proves the
 * diff shown to the operator is recomputed FRESH at review-click time (never a snapshot from
 * when the suggestion arrived), that Decline never sends a mutating command, and that the
 * confirm path runs the real action through `runAction` before — never after or in parallel
 * with — recording the outcome.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// `@/store` must be imported before any descriptor module (see `actions.test.ts`'s comment):
// the reentrant `resources/related.tsx` → `@/store` cycle otherwise leaves `ALL_DESCRIPTORS`
// with an `undefined` entry for whichever descriptor imports last.
import { initialData, listKey, useStore } from '@/store'
import { deployments } from '@/resources/deployments'
import { commandFinished, resetActions, setActionTimeout } from '@/store/actions'
import {
  actionInputFromParams,
  confirmAndExecuteSuggestedAction,
  declineSuggestedAction,
  findLiveRow,
  planSuggestedAction,
  resolveSuggestedAction,
  reviewSuggestedAction,
} from '@/components/panels/agent-ops/reviewSuggestedAction'
import type { AgentObjectRef, BackendCmd, SuggestedAction, WorkloadSummary } from '@/protocol/types'

vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), loading: vi.fn() } }))

const OBJECT: AgentObjectRef = { kind: 'Deployment', namespace: 'default', name: 'api' }

function suggestion(overrides: Partial<SuggestedAction> = {}): SuggestedAction {
  return {
    id: 'sugg-1',
    action_kind: 'scale',
    target: { kind: 'Deployment', namespace: 'default', name: 'api' },
    params: { replicas: 5 },
    rationale: 'Traffic is up; scale out.',
    status: 'Proposed',
    ...overrides,
  }
}

function dep(overrides: Partial<WorkloadSummary> = {}): WorkloadSummary {
  return { name: 'api', namespace: 'default', desired: 2, ready: 2, updated: 2, available: 2, ...overrides }
}

function must<T>(v: T | undefined, what: string): T {
  if (v === undefined) throw new Error(`missing ${what}`)
  return v
}

const DEPLOYMENTS_KEY = listKey('kind-a', deployments.section, 'Namespaced', null)

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

describe('resolveSuggestedAction', () => {
  it('resolves a matched action_kind to the real ActionDescriptor for that kind', () => {
    const resolved = resolveSuggestedAction(suggestion({ action_kind: 'scale' }))
    expect(resolved?.descriptor.kind).toBe('Deployment')
    expect(resolved?.action.id).toBe('scale')
  })

  it('resolves restart and delete too, for the kinds that actually support them', () => {
    expect(resolveSuggestedAction(suggestion({ action_kind: 'restart' }))?.action.id).toBe('restart')
    expect(resolveSuggestedAction(suggestion({ action_kind: 'delete' }))?.action.id).toBe('delete')
  })

  it('returns undefined for an action_kind with no matching ActionDescriptor on that kind', () => {
    // Pod has no 'restart' action (only 'delete') — the model inventing one must not force-fit.
    expect(
      resolveSuggestedAction(
        suggestion({ action_kind: 'restart', target: { kind: 'Pod', namespace: 'default', name: 'api-0' } }),
      ),
    ).toBeUndefined()
  })

  it('returns undefined for a target kind that has no registered descriptor at all', () => {
    expect(
      resolveSuggestedAction(
        suggestion({ target: { kind: 'TotallyMadeUpKind', namespace: 'default', name: 'x' } }),
      ),
    ).toBeUndefined()
  })
})

describe('actionInputFromParams', () => {
  it('extracts a valid replicas value', () => {
    expect(actionInputFromParams({ replicas: 7 })).toEqual({ replicas: 7 })
  })

  it('ignores non-numeric/negative/absent replicas', () => {
    expect(actionInputFromParams({ replicas: -1 })).toEqual({})
    expect(actionInputFromParams({ replicas: 'five' })).toEqual({})
    expect(actionInputFromParams({})).toEqual({})
    expect(actionInputFromParams(null)).toEqual({})
    expect(actionInputFromParams(undefined)).toEqual({})
  })
})

describe('findLiveRow', () => {
  it('finds the row for target across whatever list cache holds this kind', () => {
    useStore.setState((s) => ({
      lists: {
        ...s.lists,
        [DEPLOYMENTS_KEY]: { rows: [dep({ ready: 2 })], status: 'ready', updatedAt: 1 },
      },
    }))
    const row = findLiveRow(deployments, { kind: 'Deployment', namespace: 'default', name: 'api' })
    expect(row).toEqual(dep({ ready: 2 }))
  })

  it('returns undefined when nothing has loaded that object yet', () => {
    expect(findLiveRow(deployments, { kind: 'Deployment', namespace: 'default', name: 'api' })).toBeUndefined()
  })
})

describe('planSuggestedAction: fresh at review-time, never cached (Principle I)', () => {
  it('plan() is called against the row passed at call time, not any earlier snapshot', () => {
    const scaleDescriptor = must(
      deployments.actions.find((a) => a.id === 'scale'),
      'scale action',
    )
    const planSpy = vi.spyOn(scaleDescriptor, 'plan')
    const resolved = must(resolveSuggestedAction(suggestion()), 'resolved scale action')

    const staleRow = dep({ ready: 1, desired: 1 })
    const liveRow = dep({ ready: 4, desired: 4 })

    // Mutate the store between "suggestion arrival" and "review click": planSuggestedAction
    // must reflect whatever row is passed to it at call time, not anything captured earlier.
    const plannedStale = planSuggestedAction(resolved, suggestion(), staleRow)
    const plannedLive = planSuggestedAction(resolved, suggestion(), liveRow)

    expect(planSpy).toHaveBeenNthCalledWith(1, [staleRow], { replicas: 5 })
    expect(planSpy).toHaveBeenNthCalledWith(2, [liveRow], { replicas: 5 })
    expect(plannedStale.row).toBe(staleRow)
    expect(plannedLive.row).toBe(liveRow)
    expect(plannedStale.steps).not.toEqual(plannedLive.steps)
    planSpy.mockRestore()
  })

  it('end-to-end: mutating the store list between arrival and Review recomputes the diff', () => {
    useStore.setState((s) => ({
      lists: {
        ...s.lists,
        [DEPLOYMENTS_KEY]: { rows: [dep({ ready: 2, desired: 2 })], status: 'ready', updatedAt: 1 },
      },
    }))
    const resolved = must(resolveSuggestedAction(suggestion()), 'resolved scale action')
    const rowAtArrival = must(
      findLiveRow(resolved.descriptor, suggestion().target),
      'row at arrival',
    )
    const firstPlan = planSuggestedAction(resolved, suggestion(), rowAtArrival)
    expect(firstPlan.steps[0]?.cmd).toEqual({
      type: 'ScaleDeployment',
      data: { namespace: 'default', name: 'api', replicas: 5 },
    })

    // The store changes (a live update arrives) before the operator actually clicks Review.
    useStore.setState((s) => ({
      lists: {
        ...s.lists,
        [DEPLOYMENTS_KEY]: { rows: [dep({ ready: 3, desired: 3 })], status: 'ready', updatedAt: 2 },
      },
    }))
    const rowAtReview = must(findLiveRow(resolved.descriptor, suggestion().target), 'row at review')
    expect(rowAtReview).not.toEqual(rowAtArrival)
    const secondPlan = planSuggestedAction(resolved, suggestion(), rowAtReview)
    // The command itself is the same (scale target is fixed by the suggestion's params), but
    // the plan was computed from the freshly-read row, not the one captured at arrival time.
    expect(secondPlan.row).toEqual(dep({ ready: 3, desired: 3 }))
    expect(secondPlan.row).not.toEqual(firstPlan.row)
  })
})

describe('decline: never a mutating command', () => {
  it('sends only AgentDeclineSuggestedAction, nothing else', async () => {
    await declineSuggestedAction(OBJECT, suggestion())
    expect(sent).toEqual([
      { type: 'AgentDeclineSuggestedAction', data: { object: OBJECT, suggestion_id: 'sugg-1' } },
    ])
    expect(sent.some((c) => c.type === 'ScaleDeployment' || c.type === 'DeletePod' || c.type === 'RestartDeployment')).toBe(
      false,
    )
  })
})

describe('review: pure status bookkeeping', () => {
  it('sends only AgentReviewSuggestedAction', async () => {
    await reviewSuggestedAction(OBJECT, suggestion())
    expect(sent).toEqual([
      { type: 'AgentReviewSuggestedAction', data: { object: OBJECT, suggestion_id: 'sugg-1' } },
    ])
  })
})

describe('confirmAndExecuteSuggestedAction: runAction then AgentRecordExecutedAction, in that order', () => {
  it('success: sends the real ScaleDeployment command, then records Success only after it resolves', async () => {
    const resolved = must(resolveSuggestedAction(suggestion()), 'resolved scale action')
    const row = dep()
    const order: string[] = []

    const promise = confirmAndExecuteSuggestedAction({
      object: OBJECT,
      suggestion: suggestion(),
      descriptor: resolved.descriptor,
      action: resolved.action,
      row,
      input: { replicas: 5 },
    })

    await vi.waitFor(() => expect(sent).toHaveLength(1))
    expect(sent[0]).toEqual({
      type: 'ScaleDeployment',
      data: { namespace: 'default', name: 'api', replicas: 5 },
    })
    order.push('mutating-command-sent')

    // AgentRecordExecutedAction must not be sent yet — runAction hasn't resolved.
    expect(sent).toHaveLength(1)

    const scaleRequestId = nextId - 1
    commandFinished(scaleRequestId, { ok: true, error: null })
    await promise
    order.push('recorded')

    expect(order).toEqual(['mutating-command-sent', 'recorded'])
    expect(sent).toHaveLength(2)
    expect(sent[1]).toEqual({
      type: 'AgentRecordExecutedAction',
      data: {
        object: OBJECT,
        suggestion_id: 'sugg-1',
        command_summary: 'Scale default/api',
        outcome: { type: 'Success' },
      },
    })
  })

  it('failure: records the backend error message verbatim, never paraphrased', async () => {
    const resolved = must(
      resolveSuggestedAction(suggestion({ action_kind: 'delete' })),
      'resolved delete action',
    )
    const row = dep()

    const promise = confirmAndExecuteSuggestedAction({
      object: OBJECT,
      suggestion: suggestion({ action_kind: 'delete' }),
      descriptor: resolved.descriptor,
      action: resolved.action,
      row,
      input: {},
    })

    await vi.waitFor(() => expect(sent).toHaveLength(1))
    const requestId = nextId - 1
    commandFinished(requestId, { ok: false, error: 'deployments.apps "api" is forbidden: RBAC denies delete' })
    await promise

    expect(sent[1]).toEqual({
      type: 'AgentRecordExecutedAction',
      data: {
        object: OBJECT,
        suggestion_id: 'sugg-1',
        command_summary: 'Delete default/api',
        outcome: { type: 'Error', data: { message: 'deployments.apps "api" is forbidden: RBAC denies delete' } },
      },
    })
  })
})
