/**
 * Run approval (spec 064), the pure part: which variant of the approval panel a person sees on a
 * held run, and the list's "Waiting · k of N". The server decides again on every command; this
 * only decides what to offer (contracts/ui.md).
 *
 * - `act`: Approve and Deny.
 * - `requester`: the person who started a four-eyes run; Deny only.
 * - `answered`: they already approved; no buttons.
 * - `restricted`: no Approve in the run's workspace; who can, and no buttons.
 * - `closed`: the run left `pending_approval` (queued, denied, expired, ...); history only.
 */
import { canApprove } from '@/features/hosts/workspaces'
import type { Me, RunInfo } from '@/protocol/wire'

export type ApprovalView =
  | { kind: 'act' }
  | { kind: 'requester' }
  | { kind: 'answered' }
  | { kind: 'restricted' }
  | { kind: 'closed' }

export function approvalView({ run, me }: { run: RunInfo; me: Me | null }): ApprovalView {
  if (run.state !== 'pending_approval') return { kind: 'closed' }
  if (!canApprove(me, run.workspace)) return { kind: 'restricted' }
  if (me && run.approval?.approvals.some((a) => a.user === me.id)) return { kind: 'answered' }
  if (me && run.approval?.four_eyes && run.requested_by === me.id) return { kind: 'requester' }
  return { kind: 'act' }
}

/** A held row's state cell: "Waiting · 1 of 2". `null` for a run that is not waiting. */
export function waitingText(run: RunInfo): string | null {
  if (run.state !== 'pending_approval' || !run.approval) return null
  return `Waiting · ${String(run.approval.approvals.length)} of ${String(run.approval.needed)}`
}
