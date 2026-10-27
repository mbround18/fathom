/** The Account → Passkeys page's state: list, remove (preview, step-up, confirm). Spec 055. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { usePasskeyList } from '@/features/passkeys/list-store'
import { useSession } from '@/app/session-store'
import {
  ENROLMENT_REQUIRED as REFUSED_MESSAGES_ENROL,
  OFFLINE,
  REFUSED_MESSAGES,
  setAuthenticator,
  usePasskeys,
} from '@/features/passkeys/store'
import { toB64Url, type Authenticator, type CredentialLike } from '@/protocol/webauthn'
import type { PasskeyInfo, Preview } from '@/protocol/wire'

import { FakeSocket } from './fake-socket'
import { frames, lastCmd, ME, resetSession, signIn, timers } from './session-helpers'

const b = (...x: number[]) => new Uint8Array(x).buffer

const LAPTOP: PasskeyInfo = { id: 'AQ', name: 'Laptop', created: '2026-10-01T10:00:00Z', last_used: '2026-10-27T09:00:00Z' }
const PHONE: PasskeyInfo = { id: 'Ag', name: 'Phone', created: '2026-10-02T10:00:00Z', last_used: null }

const preview = (over: Partial<Preview> = {}): Preview => ({
  preview_id: 'pv-1',
  action: 'passkey_remove',
  summary: 'Remove the passkey “Phone”',
  affected: ['Phone'],
  step_up: false,
  expires_at: '2026-10-27T12:05:00Z',
  ...over,
})

const assertion: CredentialLike = {
  rawId: b(1),
  response: { authenticatorData: b(4), clientDataJSON: b(5), signature: b(6), userHandle: null },
}
const VERIFY_OPTIONS = { publicKey: { challenge: toB64Url(b(8)), rpId: 'localhost' } }
const get = vi.fn<Authenticator['get']>()

/** Answers the last `type` command on `s` with `events`, in order. */
function answer(s: FakeSocket, type: string, ...events: unknown[]) {
  const cmd = lastCmd(s, type)
  if (!cmd) throw new Error(`no ${type} was sent`)
  for (const event of events) s.serverSend({ reply_to: cmd.id, event })
}

function count(s: FakeSocket, type: string): number {
  return frames(s).filter((f) => f.cmd?.type === type).length
}

/** Signed in with the list loaded. */
async function loaded(items: PasskeyInfo[] = [LAPTOP, PHONE]): Promise<FakeSocket> {
  const s = signIn()
  const done = usePasskeyList.getState().load()
  answer(s, 'ListPasskeys', { type: 'Passkeys', data: { items } })
  await done
  return s
}

/** Loaded, with the removal of `PHONE` previewed by `p`. */
async function previewed(p: Preview = preview()): Promise<FakeSocket> {
  const s = await loaded()
  const started = usePasskeyList.getState().startRemove(PHONE)
  answer(s, 'PreviewPasskeyRemove', { type: 'Preview', data: p })
  await started
  return s
}

beforeEach(() => {
  resetSession()
  usePasskeys.getState().close()
  usePasskeyList.getState().reset()
  get.mockReset().mockResolvedValue(assertion)
  setAuthenticator({ create: () => Promise.resolve(null), get: (o) => get(o) })
})

afterEach(() => {
  setAuthenticator(undefined)
})

describe('list', () => {
  it('starts idle and loads the person’s passkeys', async () => {
    expect(usePasskeyList.getState()).toMatchObject({ status: 'idle', items: [] })
    const s = signIn()
    const done = usePasskeyList.getState().load()
    expect(lastCmd(s, 'ListPasskeys')?.cmd).toEqual({ type: 'ListPasskeys' })
    expect(usePasskeyList.getState().status).toBe('loading')
    answer(s, 'ListPasskeys', { type: 'Passkeys', data: { items: [LAPTOP, PHONE] } })
    await done
    expect(usePasskeyList.getState()).toMatchObject({ status: 'ready', items: [LAPTOP, PHONE], error: null })
  })

  it('shows an error the server sends', async () => {
    const s = signIn()
    const done = usePasskeyList.getState().load()
    answer(s, 'ListPasskeys', { type: 'OpError', data: { code: 'db', message: 'The database is unavailable.' } })
    await done
    expect(usePasskeyList.getState()).toMatchObject({ status: 'error', error: 'The database is unavailable.' })
  })

  it('is an error while the socket is down', async () => {
    await usePasskeyList.getState().load()
    expect(usePasskeyList.getState()).toMatchObject({ status: 'error', error: OFFLINE })
  })

  it('keeps the list on screen while it refreshes', async () => {
    const s = await loaded()
    void usePasskeyList.getState().load()
    expect(usePasskeyList.getState()).toMatchObject({ status: 'ready', items: [LAPTOP, PHONE] })
    answer(s, 'ListPasskeys', { type: 'Passkeys', data: { items: [LAPTOP] } })
    await vi.waitFor(() => expect(usePasskeyList.getState().items).toEqual([LAPTOP]))
  })

  it('only the latest load wins', async () => {
    const s = signIn()
    const first = usePasskeyList.getState().load()
    const firstId = lastCmd(s, 'ListPasskeys')?.id
    const second = usePasskeyList.getState().load()
    answer(s, 'ListPasskeys', { type: 'Passkeys', data: { items: [PHONE] } })
    s.serverSend({ reply_to: firstId, event: { type: 'Passkeys', data: { items: [LAPTOP] } } })
    await Promise.all([first, second])
    expect(usePasskeyList.getState().items).toEqual([PHONE])
  })

  it('refreshes after a passkey is registered', async () => {
    const s = await loaded([LAPTOP])
    s.serverSend({ reply_to: 99, event: { type: 'PasskeyRegistered', data: PHONE } })
    expect(count(s, 'ListPasskeys')).toBe(2)
  })

  it('does not load on a registration while the page is not open', () => {
    const s = signIn()
    s.serverSend({ reply_to: 99, event: { type: 'PasskeyRegistered', data: PHONE } })
    expect(count(s, 'ListPasskeys')).toBe(0)
  })

  it('takes a pushed Passkeys list', async () => {
    const s = await loaded([LAPTOP])
    s.serverSend({ event: { type: 'Passkeys', data: { items: [LAPTOP, PHONE] } } })
    expect(usePasskeyList.getState().items).toEqual([LAPTOP, PHONE])
  })
})

describe('remove', () => {
  it('previews the removal and waits for confirmation', async () => {
    const s = await loaded()
    const started = usePasskeyList.getState().startRemove(PHONE)
    expect(lastCmd(s, 'PreviewPasskeyRemove')?.cmd).toEqual({ type: 'PreviewPasskeyRemove', data: { id: 'Ag' } })
    expect(usePasskeyList.getState().removal).toMatchObject({ passkey: PHONE, phase: 'previewing', preview: null })
    answer(s, 'PreviewPasskeyRemove', { type: 'Preview', data: preview() })
    await started
    expect(usePasskeyList.getState().removal).toMatchObject({ phase: 'confirm', preview: preview(), error: null })
    expect(lastCmd(s, 'Confirm')).toBeUndefined()
  })

  it('refuses the last passkey with a clear message', async () => {
    const s = await loaded([LAPTOP])
    const started = usePasskeyList.getState().startRemove(LAPTOP)
    answer(s, 'PreviewPasskeyRemove', { type: 'PasskeyRefused', data: { reason: 'last_passkey' } })
    await started
    expect(usePasskeyList.getState().removal).toMatchObject({
      phase: 'refused',
      error: REFUSED_MESSAGES.last_passkey,
    })
  })

  it('confirms without step-up when the preview does not need it, then refreshes', async () => {
    const s = await previewed()
    const confirmed = usePasskeyList.getState().confirmRemove()
    expect(lastCmd(s, 'Confirm')?.cmd).toEqual({ type: 'Confirm', data: { preview_id: 'pv-1' } })
    expect(usePasskeyList.getState().removal?.phase).toBe('removing')
    expect(usePasskeys.getState().dialog).toBeNull()
    answer(s, 'Confirm', { type: 'Confirmed', data: { preview_id: 'pv-1' } }, { type: 'Passkeys', data: { items: [LAPTOP] } })
    await confirmed
    expect(usePasskeyList.getState()).toMatchObject({ removal: null, items: [LAPTOP] })
    expect(count(s, 'ListPasskeys')).toBe(1)
  })

  it('asks for a passkey first when the preview needs step-up, and carries it on Confirm', async () => {
    const s = await previewed(preview({ step_up: true }))
    const confirmed = usePasskeyList.getState().confirmRemove()
    expect(usePasskeyList.getState().removal?.phase).toBe('verifying')
    expect(usePasskeys.getState().dialog).toEqual({
      kind: 'step_up',
      mode: 'assert',
      summary: 'Remove the passkey “Phone”',
    })
    const verified = usePasskeys.getState().verify()
    answer(s, 'PasskeyBegin', { type: 'PasskeyChallenge', data: { ceremony: 'c-9', options: VERIFY_OPTIONS } })
    await verified
    await vi.waitFor(() => expect(lastCmd(s, 'Confirm')).toBeDefined())
    expect(lastCmd(s, 'Confirm')?.cmd).toEqual({
      type: 'Confirm',
      data: {
        preview_id: 'pv-1',
        step_up: {
          ceremony: 'c-9',
          credential: {
            id: 'AQ',
            rawId: 'AQ',
            response: { authenticatorData: 'BA', clientDataJSON: 'BQ', signature: 'Bg' },
            type: 'public-key',
          },
        },
      },
    })
    expect(lastCmd(s, 'PasskeyFinish')).toBeUndefined()
    expect(usePasskeyList.getState().removal?.phase).toBe('removing')
    answer(s, 'Confirm', { type: 'Confirmed', data: { preview_id: 'pv-1' } }, { type: 'Passkeys', data: { items: [LAPTOP] } })
    await confirmed
    expect(usePasskeyList.getState()).toMatchObject({ removal: null, items: [LAPTOP] })
    expect(count(s, 'ListPasskeys')).toBe(1)
  })

  it('goes back to the confirmation when step-up is cancelled, sending nothing', async () => {
    const s = await previewed(preview({ step_up: true }))
    const confirmed = usePasskeyList.getState().confirmRemove()
    usePasskeys.getState().close()
    await confirmed
    expect(usePasskeyList.getState().removal).toMatchObject({ phase: 'confirm', error: null })
    expect(lastCmd(s, 'Confirm')).toBeUndefined()
  })

  it('shows a refusal of the Confirm and refreshes the list', async () => {
    const s = await previewed()
    const confirmed = usePasskeyList.getState().confirmRemove()
    answer(s, 'Confirm', { type: 'PasskeyRefused', data: { reason: 'last_passkey' } })
    await confirmed
    expect(usePasskeyList.getState().removal).toMatchObject({ phase: 'refused', error: REFUSED_MESSAGES.last_passkey })
    expect(count(s, 'ListPasskeys')).toBe(2)
  })

  it('says to register a passkey when Confirm answers PasskeyEnrolmentRequired', async () => {
    const s = await previewed()
    const confirmed = usePasskeyList.getState().confirmRemove()
    answer(s, 'Confirm', { type: 'PasskeyEnrolmentRequired' })
    await confirmed
    expect(usePasskeyList.getState().removal).toMatchObject({ phase: 'refused', error: REFUSED_MESSAGES_ENROL })
    expect(usePasskeys.getState().dialog).toBeNull()
  })

  it('shows Denied and OpError replies verbatim', async () => {
    const s = await previewed()
    const confirmed = usePasskeyList.getState().confirmRemove()
    answer(s, 'Confirm', { type: 'OpError', data: { code: 'preview_void', message: 'That preview expired.' } })
    await confirmed
    expect(usePasskeyList.getState().removal).toMatchObject({ phase: 'refused', error: 'That preview expired.' })
  })

  it('cancel closes the removal and ignores a late answer', async () => {
    const s = await loaded()
    const started = usePasskeyList.getState().startRemove(PHONE)
    usePasskeyList.getState().cancelRemove()
    answer(s, 'PreviewPasskeyRemove', { type: 'Preview', data: preview() })
    await started
    expect(usePasskeyList.getState().removal).toBeNull()
  })

  it('does not fight a Revoked push: it stops without refreshing', async () => {
    const s = await previewed()
    const confirmed = usePasskeyList.getState().confirmRemove()
    answer(s, 'Confirm', { type: 'Confirmed', data: { preview_id: 'pv-1' } })
    s.serverSend({ event: { type: 'Revoked' } })
    await confirmed
    expect(usePasskeyList.getState()).toMatchObject({ status: 'idle', items: [], removal: null })
    expect(count(s, 'ListPasskeys')).toBe(1)
  })

  it('settles a removal left waiting when the session is revoked', async () => {
    const s = await previewed()
    const confirmed = usePasskeyList.getState().confirmRemove()
    s.serverSend({ event: { type: 'Revoked' } })
    await confirmed
    expect(usePasskeyList.getState()).toMatchObject({ status: 'idle', removal: null })
  })

  it('a dropped socket fails the waiting removal and reloads after reconnecting', async () => {
    const s = await previewed()
    const confirmed = usePasskeyList.getState().confirmRemove()
    s.serverClose(1001)
    await confirmed
    expect(usePasskeyList.getState().removal).toMatchObject({ phase: 'refused' })
    timers.shift()?.()
    const s2 = FakeSocket.last()
    expect(s2).not.toBe(s)
    s2.open()
    expect(count(s2, 'ListPasskeys')).toBe(1)
  })
})

describe('add', () => {
  const NOW = Date.parse('2026-10-27T12:00:00Z')
  const enrolled = (at: string | null) => ({
    ...ME,
    verification: { ...ME.verification, enrolled: true, verified: at !== null, at },
  })

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'], now: NOW })
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('opens the enrol dialog straight away for a person with no passkey', async () => {
    const s = signIn()
    await usePasskeyList.getState().add()
    expect(usePasskeys.getState().dialog).toEqual({ kind: 'enrol', required: false })
    expect(lastCmd(s, 'PasskeyBegin')).toBeUndefined()
  })

  it('opens the enrol dialog straight away within the step-up window', async () => {
    signIn(enrolled('2026-10-27T11:50:00Z'))
    await usePasskeyList.getState().add()
    expect(usePasskeys.getState().dialog).toEqual({ kind: 'enrol', required: false })
  })

  it('checks it is you first when the check is stale, then opens the enrol dialog', async () => {
    const s = signIn(enrolled('2026-10-27T11:00:00Z'))
    const adding = usePasskeyList.getState().add()
    expect(usePasskeys.getState().dialog).toMatchObject({ kind: 'step_up', mode: 'assert' })
    const verified = usePasskeys.getState().verify()
    answer(s, 'PasskeyBegin', { type: 'PasskeyChallenge', data: { ceremony: 'c-7', options: VERIFY_OPTIONS } })
    await verified
    await vi.waitFor(() => expect(lastCmd(s, 'PasskeyFinish')).toBeDefined())
    expect(lastCmd(s, 'PasskeyFinish')?.cmd?.data).toMatchObject({ ceremony: 'c-7' })
    const fresh = { ...enrolled('2026-10-27T12:00:00Z').verification }
    answer(s, 'PasskeyFinish', { type: 'Verification', data: fresh })
    await adding
    expect(useSession.getState().me?.verification.at).toBe('2026-10-27T12:00:00Z')
    expect(usePasskeys.getState().dialog).toEqual({ kind: 'enrol', required: false })
    expect(usePasskeyList.getState().addError).toBeNull()
  })

  it('does nothing when the check is cancelled', async () => {
    signIn(enrolled(null))
    const adding = usePasskeyList.getState().add()
    usePasskeys.getState().close()
    await adding
    expect(usePasskeys.getState().dialog).toBeNull()
    expect(usePasskeyList.getState().addError).toBeNull()
  })

  it('shows a refused check and does not open the enrol dialog', async () => {
    const s = signIn(enrolled(null))
    const adding = usePasskeyList.getState().add()
    const verified = usePasskeys.getState().verify()
    answer(s, 'PasskeyBegin', { type: 'PasskeyChallenge', data: { ceremony: 'c-7', options: VERIFY_OPTIONS } })
    await verified
    await vi.waitFor(() => expect(lastCmd(s, 'PasskeyFinish')).toBeDefined())
    answer(s, 'PasskeyFinish', { type: 'PasskeyRefused', data: { reason: 'unknown_credential' } })
    await adding
    expect(usePasskeys.getState().dialog).toBeNull()
    expect(usePasskeyList.getState().addError).toBe(REFUSED_MESSAGES.unknown_credential)
  })
})
