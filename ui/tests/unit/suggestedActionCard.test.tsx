/**
 * `SuggestedActionCard` (013-agent-sdk-ops, US3/P1 — Phase 5). Covers: status display per
 * `SuggestedActionStatus`, Review shown only for a matched `action_kind` + non-terminal status,
 * Decline dispatching `AgentDeclineSuggestedAction` and disabling once terminal, an unmatched
 * `action_kind` rendering informational-only (no Review button), and Review opening the
 * existing `ConfirmDialog` with the live diff.
 */
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { SuggestedActionCard } from '@/components/panels/agent-ops/SuggestedActionCard'
import { deployments } from '@/resources/deployments'
import { initialData, listKey, useStore } from '@/store'
import { commandFinished, resetActions, setActionTimeout } from '@/store/actions'
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

const DEP: WorkloadSummary = { name: 'api', namespace: 'default', desired: 2, ready: 2, updated: 2, available: 2 }
const DEPLOYMENTS_KEY = listKey('kind-a', deployments.section, 'Namespaced', null)

let sent: BackendCmd[]
let nextId: number

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  setActionTimeout()
  sent = []
  nextId = 100
  useStore.setState({
    request: (cmd) => {
      sent.push(cmd)
      return Promise.resolve(nextId++)
    },
    refreshSection: vi.fn(),
    lists: {
      ...useStore.getState().lists,
      [DEPLOYMENTS_KEY]: { rows: [DEP], status: 'ready', updatedAt: 1 },
    },
  })
})

describe('SuggestedActionCard', () => {
  it('shows the status, action kind, target, and rationale', () => {
    render(<SuggestedActionCard object={OBJECT} suggestion={suggestion({ status: 'Proposed' })} />)
    const card = screen.getByTestId('agent-ops-suggested-action-card')
    expect(card).toHaveTextContent('scale')
    expect(card).toHaveTextContent('default/api')
    expect(card).toHaveTextContent('Traffic is up; scale out.')
    expect(screen.getByTestId('agent-ops-suggested-action-status')).toHaveTextContent('Proposed')
  })

  it('shows a Review button for a matched action_kind while Proposed', () => {
    render(<SuggestedActionCard object={OBJECT} suggestion={suggestion({ status: 'Proposed' })} />)
    expect(screen.getByTestId('agent-ops-suggested-action-review')).toBeInTheDocument()
  })

  it('still shows Review while Reviewed (non-terminal)', () => {
    render(<SuggestedActionCard object={OBJECT} suggestion={suggestion({ status: 'Reviewed' })} />)
    expect(screen.getByTestId('agent-ops-suggested-action-review')).toBeInTheDocument()
  })

  it('hides Review once terminal (Declined/Superseded) and disables Decline', () => {
    const { rerender } = render(
      <SuggestedActionCard object={OBJECT} suggestion={suggestion({ status: 'Declined' })} />,
    )
    expect(screen.queryByTestId('agent-ops-suggested-action-review')).toBeNull()
    expect(screen.getByTestId('agent-ops-suggested-action-decline')).toBeDisabled()

    rerender(<SuggestedActionCard object={OBJECT} suggestion={suggestion({ status: 'Superseded' })} />)
    expect(screen.queryByTestId('agent-ops-suggested-action-review')).toBeNull()
    expect(screen.getByTestId('agent-ops-suggested-action-decline')).toBeDisabled()
  })

  it('renders informational-only with no Review button for an unmatched action_kind', () => {
    render(
      <SuggestedActionCard
        object={OBJECT}
        suggestion={suggestion({ action_kind: 'not-a-real-action' })}
      />,
    )
    expect(screen.queryByTestId('agent-ops-suggested-action-review')).toBeNull()
    expect(screen.getByTestId('agent-ops-suggested-action-unmatched')).toHaveTextContent(
      "isn't directly executable from this view",
    )
    // Decline is still available: declining never depends on the action being executable.
    expect(screen.getByTestId('agent-ops-suggested-action-decline')).toBeEnabled()
  })

  it('Decline dispatches AgentDeclineSuggestedAction and nothing else', async () => {
    const user = userEvent.setup()
    render(<SuggestedActionCard object={OBJECT} suggestion={suggestion()} />)
    await user.click(screen.getByTestId('agent-ops-suggested-action-decline'))
    expect(sent).toEqual([
      { type: 'AgentDeclineSuggestedAction', data: { object: OBJECT, suggestion_id: 'sugg-1' } },
    ])
  })

  it('Review sends AgentReviewSuggestedAction (Proposed→Reviewed) and opens ConfirmDialog with the live diff', async () => {
    const user = userEvent.setup()
    render(<SuggestedActionCard object={OBJECT} suggestion={suggestion({ status: 'Proposed' })} />)
    await user.click(screen.getByTestId('agent-ops-suggested-action-review'))

    expect(sent).toContainEqual({
      type: 'AgentReviewSuggestedAction',
      data: { object: OBJECT, suggestion_id: 'sugg-1' },
    })
    expect(await screen.findByTestId('confirm-dialog')).toBeVisible()
    expect(screen.getByTestId('confirm-item')).toHaveTextContent('default/api')
  })

  it('does not re-send AgentReviewSuggestedAction once already Reviewed', async () => {
    const user = userEvent.setup()
    render(<SuggestedActionCard object={OBJECT} suggestion={suggestion({ status: 'Reviewed' })} />)
    await user.click(screen.getByTestId('agent-ops-suggested-action-review'))
    expect(sent.some((c) => c.type === 'AgentReviewSuggestedAction')).toBe(false)
    expect(await screen.findByTestId('confirm-dialog')).toBeVisible()
  })

  it('confirming the dialog sends the real mutating command, then AgentRecordExecutedAction', async () => {
    const user = userEvent.setup()
    render(<SuggestedActionCard object={OBJECT} suggestion={suggestion({ status: 'Reviewed' })} />)
    await user.click(screen.getByTestId('agent-ops-suggested-action-review'))
    await screen.findByTestId('confirm-dialog')

    await user.click(screen.getByTestId('confirm-submit'))
    await vi.waitFor(() =>
      expect(sent).toContainEqual({
        type: 'ScaleDeployment',
        data: { namespace: 'default', name: 'api', replicas: 5 },
      }),
    )
    expect(screen.queryByTestId('confirm-dialog')).toBeNull()

    const requestId = nextId - 1
    commandFinished(requestId, { ok: true, error: null })
    await vi.waitFor(() =>
      expect(sent).toContainEqual({
        type: 'AgentRecordExecutedAction',
        data: {
          object: OBJECT,
          suggestion_id: 'sugg-1',
          command_summary: 'Scale default/api',
          outcome: { type: 'Success' },
        },
      }),
    )
  })

  it('cancelling the dialog sends no command at all', async () => {
    const user = userEvent.setup()
    render(<SuggestedActionCard object={OBJECT} suggestion={suggestion({ status: 'Reviewed' })} />)
    await user.click(screen.getByTestId('agent-ops-suggested-action-review'))
    await screen.findByTestId('confirm-dialog')
    await user.click(screen.getByTestId('confirm-cancel'))
    expect(screen.queryByTestId('confirm-dialog')).toBeNull()
    expect(sent).toEqual([])
  })

  it('shows the recorded outcome once executed', () => {
    render(
      <SuggestedActionCard
        object={OBJECT}
        suggestion={suggestion({ status: 'Reviewed' })}
        executedOutcome={{ type: 'Error', data: { message: 'quota exceeded' } }}
      />,
    )
    expect(screen.getByTestId('agent-ops-suggested-action-outcome')).toHaveTextContent(
      'quota exceeded',
    )
  })
})
