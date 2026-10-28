import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { setAuthenticator } from '@/features/passkeys/store'
import { commentOk, shortSha, useReviews } from '@/features/reviews/store'
import { toB64Url, type Authenticator, type CredentialLike } from '@/protocol/webauthn'
import type { ReviewInfo } from '@/protocol/wire'

import type { FakeSocket } from './fake-socket'
import { lastCmd, resetSession, signIn } from './session-helpers'

const b = (...x: number[]) => new Uint8Array(x).buffer
const assertion: CredentialLike = {
  rawId: b(7),
  response: { authenticatorData: b(4), clientDataJSON: b(5), signature: b(6), userHandle: null },
}
const get = vi.fn<Authenticator['get']>()
const auth: Authenticator = { create: () => Promise.resolve(null), get: (o) => get(o) }

function answer(s: FakeSocket, type: string, ...events: unknown[]) {
  const cmd = lastCmd(s, type)
  if (!cmd) throw new Error(`no ${type} was sent`)
  for (const event of events) s.serverSend({ reply_to: cmd.id, event })
}

const review = (over: Partial<ReviewInfo> = {}): ReviewInfo => ({
  id: 'g-1',
  run: 'r-1',
  workspace: 'w-1',
  binding: 'b-1',
  job_name: 'web',
  base: null,
  target: 'a'.repeat(40),
  hosts: ['web-1'],
  reason: 'baseline',
  state: 'open',
  approvals: 0,
  quorum: 1,
  four_eyes: false,
  flagged: 0,
  created_at: '2026-10-27T12:00:00Z',
  ...over,
})

const preview = {
  preview_id: 'p-1',
  action: 'preview_override',
  summary: 'Override web',
  affected: ['web-1'],
  step_up: true,
  expires_at: '2026-10-27T12:15:00Z',
}
const challenge = {
  preview_id: 'p-1',
  request: { ceremony: 'cer-1', options: { publicKey: { challenge: toB64Url(b(9)), rpId: 'localhost' } } },
  canonical: '{"run":"r-1"}',
  expires_at: '2026-10-28T12:00:00Z',
}

beforeEach(() => {
  resetSession()
  useReviews.setState({
    workspace: null,
    filter: 'open',
    items: [],
    selected: null,
    detail: null,
    files: [],
    diff: null,
    pending: null,
    signing: false,
    signers: [],
    error: null,
  })
  get.mockReset().mockResolvedValue(assertion)
  setAuthenticator(auth)
})

afterEach(() => {
  setAuthenticator(undefined)
})

describe('helpers', () => {
  it('needs a comment that is not blank and fits', () => {
    expect(commentOk('')).toBe(false)
    expect(commentOk('   ')).toBe(false)
    expect(commentOk('ok')).toBe(true)
    expect(commentOk('x'.repeat(2000))).toBe(true)
    expect(commentOk('x'.repeat(2001))).toBe(false)
  })
  it('shortens commits', () => {
    expect(shortSha('0123456789abcdef')).toBe('0123456789ab')
    expect(shortSha(null)).toBe('none')
  })
})

describe('reviews', () => {
  it('lists, opens a group and its diff, and pages a file', () => {
    const s = signIn()
    useReviews.getState().load('w-1')
    expect(lastCmd(s, 'ListReviews')?.cmd?.data).toEqual({ workspace: 'w-1', state: 'open' })
    expect(lastCmd(s, 'ListSigners')?.cmd?.data).toEqual({ workspace: 'w-1' })
    answer(s, 'ListReviews', { type: 'Reviews', data: { items: [review()] } })
    expect(useReviews.getState().items).toHaveLength(1)

    useReviews.getState().select('g-1')
    answer(s, 'GetReview', { type: 'Review', data: { review: review(), commits: [], flags: [], decisions: [] } })
    answer(s, 'GetReviewDiff', {
      type: 'ReviewDiff',
      data: { group: 'g-1', files: [{ path: 'a.yml', status: 'added', flags: [] }], page: null },
    })
    expect(useReviews.getState().detail?.review.id).toBe('g-1')
    expect(useReviews.getState().files).toHaveLength(1)

    useReviews.getState().openFile('a.yml', 1)
    expect(lastCmd(s, 'GetReviewDiff')?.cmd?.data).toEqual({ group: 'g-1', path: 'a.yml', page: 1 })

    // A pushed request re-lists.
    s.serverSend({ event: { type: 'ReviewRequested', data: { group: 'g-2', run: 'r-2', workspace: 'w-1' } } })
    expect(s.sent.filter((f) => f.includes('"ListReviews"'))).toHaveLength(2)
  })

  it('refuses to approve without a comment', () => {
    const s = signIn()
    useReviews.setState({ workspace: 'w-1', selected: 'g-1' })
    useReviews.getState().approve('  ')
    expect(lastCmd(s, 'PreviewOverride')).toBeUndefined()
  })

  it('approves by signing the override challenge, never with a step-up alone', async () => {
    const s = signIn()
    useReviews.setState({ workspace: 'w-1', selected: 'g-1', items: [review()] })
    useReviews.getState().approve('Looked at the diff.')
    expect(lastCmd(s, 'PreviewOverride')?.cmd?.data).toEqual({ group: 'g-1', comment: 'Looked at the diff.' })
    answer(s, 'PreviewOverride', { type: 'Preview', data: preview }, { type: 'OverrideChallenge', data: challenge })
    expect(useReviews.getState().pending).toMatchObject({ kind: 'approve', challenge: { canonical: '{"run":"r-1"}' } })

    await useReviews.getState().confirm()
    expect(get).toHaveBeenCalledOnce()
    const confirm = lastCmd(s, 'Confirm')?.cmd?.data as { preview_id: string; step_up: { ceremony: string } }
    expect(confirm.preview_id).toBe('p-1')
    expect(confirm.step_up.ceremony).toBe('cer-1')

    answer(
      s,
      'Confirm',
      { type: 'Confirmed', data: { preview_id: 'p-1' } },
      { type: 'Review', data: { review: review({ state: 'approved', approvals: 1 }), commits: [], flags: [], decisions: [] } },
    )
    expect(useReviews.getState().pending).toBeNull()
    expect(useReviews.getState().items[0]?.state).toBe('approved')
  })

  it('a cancelled passkey prompt sends nothing', async () => {
    const s = signIn()
    get.mockRejectedValue(new Error('NotAllowedError'))
    useReviews.setState({ workspace: 'w-1', selected: 'g-1' })
    useReviews.getState().approve('ok')
    answer(s, 'PreviewOverride', { type: 'Preview', data: preview }, { type: 'OverrideChallenge', data: challenge })
    await useReviews.getState().confirm()
    expect(lastCmd(s, 'Confirm')).toBeUndefined()
    expect(useReviews.getState().error).toContain('cancelled')
  })
})
