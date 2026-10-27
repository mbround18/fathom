import { beforeEach, describe, expect, it, vi } from 'vitest'

import { usePasskeys } from '@/features/passkeys/store'
import { useGovernance } from '@/features/teams/store'
import type { PasskeyAssertion, Preview } from '@/protocol/wire'

import type { FakeSocket } from './fake-socket'
import { frames, lastCmd, ME, resetSession, signIn } from './session-helpers'

function answer(s: FakeSocket, type: string, ...events: unknown[]) {
  const cmd = lastCmd(s, type)
  if (!cmd) throw new Error(`no ${type} was sent`)
  for (const event of events) s.serverSend({ reply_to: cmd.id, event })
}

const preview = (over: Partial<Preview> = {}): Preview => ({
  preview_id: 'p-1',
  action: 'grant_add',
  summary: 'Give Platform operator on payments',
  affected: ['Ada Lovelace', 'Grace Hopper'],
  step_up: false,
  expires_at: '2026-10-27T12:15:00Z',
  ...over,
})

const ASSERTION = {
  ceremony: 'c-1',
  credential: {
    id: 'Bw',
    rawId: 'Bw',
    response: { authenticatorData: 'BA', clientDataJSON: 'BQ', signature: 'Bg', userHandle: null },
    type: 'public-key',
  },
} as unknown as PasskeyAssertion

const GRANT = {
  type: 'PreviewGrantAdd',
  data: { team: 't-1', workspace: 'w-1', role: 'operator', profiles: [] },
} as const

beforeEach(() => {
  resetSession()
  useGovernance.getState().dismiss()
  useGovernance.setState({ workspaces: null, teams: null, users: null, tokens: null, refused: {} })
  usePasskeys.getState().close()
})

describe('lists', () => {
  it('keeps each list, and a refusal verbatim in its place', () => {
    const s = signIn()
    const g = useGovernance.getState()
    g.load('teams')
    g.load('users')
    answer(s, 'ListTeams', { type: 'Teams', data: { items: [] } })
    answer(s, 'ListUsers', { type: 'Denied', data: { action: 'list_users', reason: 'instance_admin_only' } })
    expect(useGovernance.getState().teams).toEqual([])
    expect(useGovernance.getState().users).toBeNull()
    expect(useGovernance.getState().refused.users).toEqual({
      kind: 'denied',
      action: 'list_users',
      reason: 'instance_admin_only',
    })
  })
})

describe('changes', () => {
  it('previews, confirms, and reloads the lists on show', () => {
    const s = signIn()
    useGovernance.getState().load('teams')
    answer(s, 'ListTeams', { type: 'Teams', data: { items: [] } })

    useGovernance.getState().propose('Add grant', GRANT)
    expect(lastCmd(s, 'PreviewGrantAdd')?.cmd?.data).toEqual(GRANT.data)
    expect(useGovernance.getState().change?.phase).toBe('asking')

    answer(s, 'PreviewGrantAdd', { type: 'Preview', data: preview() })
    expect(useGovernance.getState().change?.preview?.affected).toEqual(['Ada Lovelace', 'Grace Hopper'])

    void useGovernance.getState().confirm()
    expect(lastCmd(s, 'Confirm')?.cmd?.data).toEqual({ preview_id: 'p-1' })
    const before = frames(s).filter((f) => f.cmd?.type === 'ListTeams').length
    answer(s, 'Confirm', { type: 'Confirmed', data: { preview_id: 'p-1' } })
    expect(useGovernance.getState().change?.phase).toBe('done')
    expect(frames(s).filter((f) => f.cmd?.type === 'ListTeams').length).toBe(before + 1)
  })

  it('shows a Denied preview verbatim and confirms nothing', () => {
    const s = signIn()
    useGovernance.getState().propose('Add grant', GRANT)
    answer(s, 'PreviewGrantAdd', { type: 'Denied', data: { action: 'preview_grant_add', reason: 'no_grant' } })
    const c = useGovernance.getState().change
    expect(c?.phase).toBe('refused')
    expect(c?.refusal).toEqual({ kind: 'denied', action: 'preview_grant_add', reason: 'no_grant' })
    void useGovernance.getState().confirm()
    expect(lastCmd(s, 'Confirm')).toBeUndefined()
  })

  it('asks for a passkey when the preview needs step-up and the session has not verified', async () => {
    const s = signIn({ ...ME, verification: { ...ME.verification, enrolled: true } })
    const requestStepUp = vi.fn().mockResolvedValue(ASSERTION)
    usePasskeys.setState({ requestStepUp })
    useGovernance.getState().propose('Add grant', GRANT)
    answer(s, 'PreviewGrantAdd', { type: 'Preview', data: preview({ step_up: true }) })

    await useGovernance.getState().confirm()
    expect(requestStepUp).toHaveBeenCalledWith('Give Platform operator on payments')
    expect(lastCmd(s, 'Confirm')?.cmd?.data).toEqual({ preview_id: 'p-1', step_up: ASSERTION })
  })

  it('a cancelled passkey prompt goes back to the preview without confirming', async () => {
    const s = signIn({ ...ME, verification: { ...ME.verification, enrolled: true } })
    usePasskeys.setState({ requestStepUp: vi.fn().mockResolvedValue(null) })
    useGovernance.getState().propose('Add grant', GRANT)
    answer(s, 'PreviewGrantAdd', { type: 'Preview', data: preview({ step_up: true }) })
    await useGovernance.getState().confirm()
    expect(useGovernance.getState().change?.phase).toBe('preview')
    expect(lastCmd(s, 'Confirm')).toBeUndefined()
  })

  it('a session verified within its window confirms without a new assertion', async () => {
    const s = signIn({ ...ME, verification: { ...ME.verification, enrolled: true, verified: true } })
    const requestStepUp = vi.fn()
    usePasskeys.setState({ requestStepUp })
    useGovernance.getState().propose('Add grant', GRANT)
    answer(s, 'PreviewGrantAdd', { type: 'Preview', data: preview({ step_up: true }) })
    await useGovernance.getState().confirm()
    expect(requestStepUp).not.toHaveBeenCalled()
    expect(lastCmd(s, 'Confirm')?.cmd?.data).toEqual({ preview_id: 'p-1' })
  })

  it('no passkey enrolled: the refusal says so and opens enrolment', async () => {
    const s = signIn()
    useGovernance.getState().propose('Add grant', GRANT)
    answer(s, 'PreviewGrantAdd', { type: 'Preview', data: preview({ step_up: true }) })
    await useGovernance.getState().confirm()
    answer(s, 'Confirm', { type: 'PasskeyEnrolmentRequired' })
    expect(useGovernance.getState().change?.refusal).toMatchObject({ code: 'passkey_required' })
    expect(usePasskeys.getState().dialog).toEqual({ kind: 'enrol', required: true })
  })

  it('keeps a new token secret only until the dialog closes', async () => {
    const s = signIn({ ...ME, verification: { ...ME.verification, verified: true } })
    useGovernance.getState().propose('New API token', {
      type: 'PreviewApiTokenCreate',
      data: { name: 'ci', workspace: 'w-1', actions: ['list_teams'], expires_in_days: 30 },
    })
    answer(s, 'PreviewApiTokenCreate', { type: 'Preview', data: preview({ action: 'api_token_create', step_up: true }) })
    await useGovernance.getState().confirm()
    const token = {
      id: 'k-1',
      name: 'ci',
      owner: 'u-1',
      workspace: 'w-1',
      workspace_slug: 'payments',
      actions: ['list_teams'],
      expires_at: '2026-11-26T12:00:00Z',
      last_used_at: null,
      revoked: false,
    }
    answer(
      s,
      'Confirm',
      { type: 'Confirmed', data: { preview_id: 'p-1' } },
      { type: 'ApiTokenCreated', data: { token, secret: 'kt_secret' } },
    )
    expect(useGovernance.getState().change?.created?.secret).toBe('kt_secret')
    useGovernance.getState().dismiss()
    expect(useGovernance.getState().change).toBeNull()
  })

  it('ignores a reply for a change that was dismissed', () => {
    const s = signIn()
    useGovernance.getState().propose('Add grant', GRANT)
    useGovernance.getState().dismiss()
    answer(s, 'PreviewGrantAdd', { type: 'Preview', data: preview() })
    expect(useGovernance.getState().change).toBeNull()
  })
})
