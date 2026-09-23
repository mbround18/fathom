/**
 * One `SuggestedAction` card (013-agent-sdk-ops, US3/P1 — Phase 5, constitution Principle I).
 *
 * Review opens the EXISTING `ConfirmDialog` (same component/testids every manual mutating
 * action uses) with a diff computed FRESH at click time — never anything cached from when the
 * suggestion arrived. Confirming it sends the SAME unmodified command(s) a manual click would,
 * through the SAME `runAction`, and only records the outcome once that has actually resolved.
 * Decline never touches the cluster at all. See `reviewSuggestedAction.ts` for the adapter this
 * card is a thin UI shell around.
 */
import { useState } from 'react'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Button } from '@/components/ui/button'
import type { AgentObjectRef, ExecutedAction, SuggestedAction } from '@/protocol/types'

import {
  confirmAndExecuteSuggestedAction,
  declineSuggestedAction,
  findLiveRow,
  planSuggestedAction,
  resolveSuggestedAction,
  reviewSuggestedAction,
  type PlannedAction,
} from './reviewSuggestedAction'

const TERMINAL_STATUSES: readonly SuggestedAction['status'][] = ['Declined', 'Superseded']

export function SuggestedActionCard({
  object,
  suggestion,
  executedOutcome,
}: {
  object: AgentObjectRef
  suggestion: SuggestedAction
  /** From `AgentConversation.executed_actions`, once this suggestion has been executed. */
  executedOutcome?: ExecutedAction['outcome']
}) {
  const resolved = resolveSuggestedAction(suggestion)
  const [planned, setPlanned] = useState<PlannedAction | undefined>(undefined)
  const [notice, setNotice] = useState<string | undefined>(undefined)
  const [busy, setBusy] = useState(false)
  const terminal = TERMINAL_STATUSES.includes(suggestion.status)

  const onReview = () => {
    setNotice(undefined)
    if (!resolved) return
    if (suggestion.status === 'Proposed') {
      // Pure status bookkeeping (FR-007) — fire and forget, never gates opening the dialog.
      void reviewSuggestedAction(object, suggestion)
    }
    const row = findLiveRow(resolved.descriptor, suggestion.target)
    if (!row) {
      setNotice(
        `${resolved.descriptor.kind} ${suggestion.target.namespace}/${suggestion.target.name} ` +
          "isn't loaded in any open list right now — open it to review this action.",
      )
      return
    }
    // Fresh, at click time: never a plan computed earlier or cached from suggestion-creation.
    const plan = planSuggestedAction(resolved, suggestion, row)
    if (plan.steps.length === 0) {
      setNotice('This suggested action has no effect to apply.')
      return
    }
    setPlanned(plan)
  }

  const onDecline = () => {
    void declineSuggestedAction(object, suggestion)
  }

  const onCancel = () => {
    setPlanned(undefined)
  }

  const onConfirm = () => {
    if (!planned || !resolved) return
    const { descriptor, action } = resolved
    const row = planned.row
    const input = planned.input
    setPlanned(undefined)
    setBusy(true)
    void confirmAndExecuteSuggestedAction({
      object,
      suggestion,
      descriptor,
      action,
      row,
      input,
    }).finally(() => setBusy(false))
  }

  const items = planned
    ? [
        {
          namespace: resolved?.descriptor.ref(planned.row).namespace ?? null,
          name: resolved?.descriptor.ref(planned.row).name ?? '',
        },
      ]
    : []

  return (
    <div
      data-testid="agent-ops-suggested-action-card"
      data-status={suggestion.status}
      className="rounded-md border border-accent-neon/40 bg-accent-neon/5 p-2 text-xs"
    >
      <p className="font-display text-[10px] tracking-wider text-accent-neon uppercase">
        {suggestion.action_kind} · {suggestion.target.kind} {suggestion.target.namespace}/
        {suggestion.target.name}
      </p>
      <p className="pt-1 text-foreground/80">{suggestion.rationale}</p>
      <p
        data-testid="agent-ops-suggested-action-status"
        className="pt-1 text-[10px] tracking-wider text-muted-foreground uppercase"
      >
        {suggestion.status}
      </p>
      {executedOutcome && (
        <p data-testid="agent-ops-suggested-action-outcome" className="pt-1 text-foreground/80">
          {executedOutcome.type === 'Success'
            ? 'Executed: succeeded.'
            : `Executed: failed — ${executedOutcome.data.message}`}
        </p>
      )}
      {!resolved && (
        <p data-testid="agent-ops-suggested-action-unmatched" className="pt-1 text-muted-foreground">
          This suggested action type isn't directly executable from this view.
        </p>
      )}
      <div className="flex gap-2 pt-2">
        {resolved && !terminal && (
          <Button
            type="button"
            size="sm"
            variant="outline"
            data-testid="agent-ops-suggested-action-review"
            disabled={busy}
            onClick={onReview}
          >
            Review
          </Button>
        )}
        <Button
          type="button"
          size="sm"
          variant="ghost"
          data-testid="agent-ops-suggested-action-decline"
          disabled={terminal || busy}
          onClick={onDecline}
        >
          Decline
        </Button>
      </div>
      {notice && (
        <p data-testid="agent-ops-suggested-action-notice" className="pt-1 text-muted-foreground">
          {notice}
        </p>
      )}
      {planned && resolved && (
        <ConfirmDialog
          open
          title={`${resolved.action.label} ${resolved.descriptor.kind.toLowerCase()}?`}
          items={items}
          destructive={resolved.action.destructive ?? false}
          confirmLabel={`${resolved.action.label} ${resolved.descriptor.kind.toLowerCase()}`}
          onConfirm={onConfirm}
          onCancel={onCancel}
        />
      )}
    </div>
  )
}
