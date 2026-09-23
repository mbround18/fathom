/**
 * The Review/Confirm/Decline adapter for `SuggestedAction`s (013-agent-sdk-ops, US3/P1 —
 * Phase 5). This is the safety-critical file for the whole feature: constitution Principle I
 * says every AI-suggested action is inert until a human explicitly confirms it through the
 * EXACT SAME diff-preview + confirmation-gated mutation path a human-initiated action would
 * use, recomputed fresh at confirm-time, never auto-applied.
 *
 * There is deliberately no new mutation command here. Confirming a suggestion means sending
 * the same, unmodified, pre-existing mutating `BackendCmd` the matching `ActionDescriptor`
 * already builds (`ui/src/resources/actions.ts`), through the same `runAction`
 * (`ui/src/store/actions.ts`) every manual button click uses — see
 * `confirmAndExecuteSuggestedAction` below for the one call site that is allowed to trigger a
 * cluster mutation from a suggestion.
 */
import type { StoreApi } from 'zustand'

import type {
  ActionDescriptor,
  ActionInput,
  ActionStep,
  ResourceDescriptor,
} from '@/resources'
import { ALL_DESCRIPTORS } from '@/resources'
import type { ActionOutcome, AgentObjectRef, SuggestedAction } from '@/protocol/types'
import { objectName, runAction, ACTION_TIMEOUT_MS } from '@/store/actions'
import { useStore, type ListRow, type StoreState } from '@/store'

export interface ResolvedAction {
  descriptor: ResourceDescriptor<ListRow>
  action: ActionDescriptor<ListRow>
}

/**
 * Resolves `suggestion.action_kind` against the SAME `ActionDescriptor` list a manual click on
 * this resource kind's menu would use: find the `ResourceDescriptor` whose `kind` matches
 * `suggestion.target.kind`, then the `ActionDescriptor` in its `.actions` whose `id` matches
 * `suggestion.action_kind`. Returns `undefined` (never a made-up/best-guess match) when either
 * lookup fails — e.g. the target kind isn't a registered resource, or the model invented an
 * `action_kind` no `ActionDescriptor` on that kind actually has (contracts.md rule 1: render
 * informational-only, do not force-fit).
 */
export function resolveSuggestedAction(suggestion: SuggestedAction): ResolvedAction | undefined {
  const descriptor = ALL_DESCRIPTORS.find((d) => d.kind === suggestion.target.kind)
  if (!descriptor) return undefined
  const action = descriptor.actions.find((a) => a.id === suggestion.action_kind)
  if (!action) return undefined
  return { descriptor, action }
}

/**
 * Finds the CURRENT live row for `target` among whatever this resource kind's lists have
 * already loaded into the store — scanning every cached list key for `descriptor.section`
 * (namespace filter/context vary the key, so this is not a single lookup). Returns `undefined`
 * if nothing has loaded that object yet; callers must not fall back to any cached/stale row.
 */
export function findLiveRow(
  descriptor: ResourceDescriptor<ListRow>,
  target: AgentObjectRef,
  store: StoreApi<StoreState> = useStore,
): ListRow | undefined {
  const lists = store.getState().lists
  const marker = `|${descriptor.section}|`
  for (const key of Object.keys(lists)) {
    if (!key.includes(marker)) continue
    const list = lists[key]
    if (!list) continue
    for (const row of list.rows) {
      const ref = descriptor.ref(row)
      if ((ref.namespace ?? '') === target.namespace && ref.name === target.name) return row
    }
  }
  return undefined
}

/** Best-effort extraction of `ActionInput` from a `SuggestedAction`'s opaque `params`. */
export function actionInputFromParams(params: unknown): ActionInput {
  if (params && typeof params === 'object' && !Array.isArray(params)) {
    const replicas = (params as Record<string, unknown>).replicas
    if (typeof replicas === 'number' && Number.isInteger(replicas) && replicas >= 0) {
      return { replicas }
    }
  }
  return {}
}

export interface PlannedAction {
  row: ListRow
  input: ActionInput
  steps: ActionStep<ListRow>[]
}

/**
 * Computes a FRESH plan for `suggestion` from the CURRENT `row` passed in — this must always be
 * a row just read out of the store (`findLiveRow`), never anything cached from when the
 * suggestion first arrived. Calling `resolved.action.plan()` here, at review-click time, is
 * what makes the diff the operator sees in `ConfirmDialog` reflect live cluster state, exactly
 * as a manual action click would (nothing about the plan is computed or cached earlier).
 */
export function planSuggestedAction(
  resolved: ResolvedAction,
  suggestion: SuggestedAction,
  row: ListRow,
): PlannedAction {
  const input = actionInputFromParams(suggestion.params)
  const steps = resolved.action.plan([row], input)
  return { row, input, steps }
}

export interface ConfirmExecuteInput {
  object: AgentObjectRef
  suggestion: SuggestedAction
  descriptor: ResourceDescriptor<ListRow>
  action: ActionDescriptor<ListRow>
  row: ListRow
  input: ActionInput
  store?: StoreApi<StoreState>
}

/**
 * `ConfirmDialog`'s `onConfirm` calls this, and only this. It runs the exact command(s) a
 * manual click would through `runAction`, waits for the real result, and only then records the
 * outcome as local bookkeeping via `AgentRecordExecutedAction` — never before, never in
 * parallel, and never with a paraphrased error.
 */
export async function confirmAndExecuteSuggestedAction(ctx: ConfirmExecuteInput): Promise<void> {
  const store = ctx.store ?? useStore

  // PRINCIPLE I ENFORCEMENT POINT: this is the one and only place a suggested action can
  // cause a cluster mutation. It runs through the exact same runAction()/ActionDescriptor.plan()
  // path a manual button click uses, recomputed fresh above — never a separate/shortcut path.
  // See .specify/memory/constitution.md Principle I.
  const outcome = await runAction(ctx.descriptor, ctx.action, [ctx.row], {
    input: ctx.input,
    store,
  })

  const actionOutcome: ActionOutcome =
    outcome.failed.length > 0
      ? { type: 'Error', data: { message: outcome.failed[0]?.error ?? 'The action failed.' } }
      : outcome.timedOut.length > 0
        ? {
            type: 'Error',
            data: {
              message: `No response from backend after ${String(Math.round(ACTION_TIMEOUT_MS / 1000))}s.`,
            },
          }
        : { type: 'Success' }

  // Only recorded AFTER runAction has actually resolved — this is pure local bookkeeping of a
  // command that already ran, never a trigger for one.
  await store.getState().request({
    type: 'AgentRecordExecutedAction',
    data: {
      object: ctx.object,
      suggestion_id: ctx.suggestion.id,
      command_summary: `${ctx.action.label} ${objectName(ctx.descriptor, ctx.row)}`,
      outcome: actionOutcome,
    },
  })
}

/** Proposed → Reviewed. Never mutates anything cluster-side (FR-007): pure status bookkeeping. */
export function reviewSuggestedAction(
  object: AgentObjectRef,
  suggestion: SuggestedAction,
  store: StoreApi<StoreState> = useStore,
): Promise<number> {
  return store.getState().request({
    type: 'AgentReviewSuggestedAction',
    data: { object, suggestion_id: suggestion.id },
  })
}

/** Declines a suggestion. Never sends a mutating command, only this bookkeeping one. */
export function declineSuggestedAction(
  object: AgentObjectRef,
  suggestion: SuggestedAction,
  store: StoreApi<StoreState> = useStore,
): Promise<number> {
  return store.getState().request({
    type: 'AgentDeclineSuggestedAction',
    data: { object, suggestion_id: suggestion.id },
  })
}
