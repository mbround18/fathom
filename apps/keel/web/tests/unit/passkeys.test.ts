import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useSession } from '@/app/session-store'
import { setAuthenticator, usePasskeys } from '@/features/passkeys/store'
import { toB64Url, type Authenticator, type CredentialLike } from '@/protocol/webauthn'

import type { FakeSocket } from './fake-socket'
import { lastCmd, ME, resetSession, signIn } from './session-helpers'

const b = (...x: number[]) => new Uint8Array(x).buffer

const REGISTER_OPTIONS = {
  publicKey: {
    rp: { id: 'localhost', name: 'Keel' },
    user: { id: toB64Url(b(1)), name: 'ada', displayName: 'Ada' },
    challenge: toB64Url(b(9)),
    pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
  },
}
const VERIFY_OPTIONS = { publicKey: { challenge: toB64Url(b(8)), rpId: 'localhost' } }

const attestation: CredentialLike = {
  rawId: b(7),
  response: { attestationObject: b(2), clientDataJSON: b(3), getTransports: () => ['internal'] },
}
const assertion: CredentialLike = {
  rawId: b(7),
  response: { authenticatorData: b(4), clientDataJSON: b(5), signature: b(6), userHandle: null },
}

const create = vi.fn<Authenticator['create']>()
const get = vi.fn<Authenticator['get']>()
const auth: Authenticator = { create: (o) => create(o), get: (o) => get(o) }

/** Answers the last `type` command on `s` with `event`. */
function answer(s: FakeSocket, type: string, event: unknown) {
  const cmd = lastCmd(s, type)
  if (!cmd) throw new Error(`no ${type} was sent`)
  s.serverSend({ reply_to: cmd.id, event })
}

beforeEach(() => {
  resetSession()
  usePasskeys.getState().close()
  create.mockReset().mockResolvedValue(attestation)
  get.mockReset().mockResolvedValue(assertion)
  setAuthenticator(auth)
})

afterEach(() => {
  setAuthenticator(undefined)
})

describe('enrol', () => {
  it('registers: begin, authenticator, finish, registered', async () => {
    const s = signIn()
    usePasskeys.getState().openEnrol()
    const done = usePasskeys.getState().enrol(' Laptop ')
    expect(lastCmd(s, 'PasskeyBegin')?.cmd?.data).toEqual({ purpose: { kind: 'register', name: 'Laptop' } })
    expect(usePasskeys.getState().phase).toBe('server')

    answer(s, 'PasskeyBegin', { type: 'PasskeyChallenge', data: { ceremony: 'c-1', options: REGISTER_OPTIONS } })
    await vi.waitFor(() => expect(lastCmd(s, 'PasskeyFinish')).toBeDefined())
    expect(create).toHaveBeenCalledOnce()
    expect(lastCmd(s, 'PasskeyFinish')?.cmd?.data).toEqual({
      ceremony: 'c-1',
      credential: {
        id: 'Bw',
        rawId: 'Bw',
        response: { attestationObject: 'Ag', clientDataJSON: 'Aw', transports: ['internal'] },
        type: 'public-key',
      },
    })

    answer(s, 'PasskeyFinish', {
      type: 'PasskeyRegistered',
      data: { id: 'Bw', name: 'Laptop', created: '2026-10-27T10:00:00Z', last_used: null },
    })
    await done
    expect(usePasskeys.getState().phase).toBe('done')
    expect(useSession.getState().me?.verification.enrolled).toBe(true)
  })

  it('shows the refusal and stays open to retry', async () => {
    const s = signIn()
    usePasskeys.getState().openEnrol()
    const done = usePasskeys.getState().enrol('Laptop')
    answer(s, 'PasskeyBegin', { type: 'PasskeyRefused', data: { reason: 'expired' } })
    await done
    expect(usePasskeys.getState()).toMatchObject({
      dialog: { kind: 'enrol' },
      phase: 'refused',
      error: 'The request took too long and expired. Try again.',
    })
  })

  it('treats a cancelled browser prompt as cancelled, without finishing', async () => {
    create.mockRejectedValueOnce(new DOMException('no', 'NotAllowedError'))
    const s = signIn()
    usePasskeys.getState().openEnrol()
    const done = usePasskeys.getState().enrol('Laptop')
    answer(s, 'PasskeyBegin', { type: 'PasskeyChallenge', data: { ceremony: 'c-1', options: REGISTER_OPTIONS } })
    await done
    expect(usePasskeys.getState().error).toBe('The passkey prompt was cancelled.')
    expect(lastCmd(s, 'PasskeyFinish')).toBeUndefined()
  })

  it('opens as required on a pushed PasskeyEnrolmentRequired', () => {
    const s = signIn({ ...ME, verification: { ...ME.verification, required: true } })
    s.serverSend({ event: { type: 'PasskeyEnrolmentRequired' } })
    expect(usePasskeys.getState().dialog).toEqual({ kind: 'enrol', required: true })
  })
})

describe('step-up', () => {
  it('verifies the session: begin, get, finish, Verification', async () => {
    const s = signIn()
    usePasskeys.getState().openStepUp()
    const done = usePasskeys.getState().verify()
    expect(lastCmd(s, 'PasskeyBegin')?.cmd?.data).toEqual({ purpose: { kind: 'verify' } })
    answer(s, 'PasskeyBegin', { type: 'PasskeyChallenge', data: { ceremony: 'c-2', options: VERIFY_OPTIONS } })
    await vi.waitFor(() => expect(lastCmd(s, 'PasskeyFinish')).toBeDefined())
    const v = { ...ME.verification, verified: true, enrolled: true, at: '2026-10-27T10:00:00Z' }
    answer(s, 'PasskeyFinish', { type: 'Verification', data: v })
    await done
    expect(usePasskeys.getState().phase).toBe('done')
    expect(useSession.getState().me?.verification.verified).toBe(true)
  })

  it('requestStepUp resolves with the assertion and sends no finish', async () => {
    const s = signIn()
    const result = usePasskeys.getState().requestStepUp('Approve run 42')
    expect(usePasskeys.getState().dialog).toEqual({ kind: 'step_up', mode: 'assert', summary: 'Approve run 42' })
    const done = usePasskeys.getState().verify()
    answer(s, 'PasskeyBegin', { type: 'PasskeyChallenge', data: { ceremony: 'c-3', options: VERIFY_OPTIONS } })
    await done
    await expect(result).resolves.toEqual({
      ceremony: 'c-3',
      credential: {
        id: 'Bw',
        rawId: 'Bw',
        response: { authenticatorData: 'BA', clientDataJSON: 'BQ', signature: 'Bg' },
        type: 'public-key',
      },
    })
    expect(lastCmd(s, 'PasskeyFinish')).toBeUndefined()
    expect(usePasskeys.getState().dialog).toBeNull()
  })

  it('requestStepUp resolves null when the dialog closes', async () => {
    signIn()
    const result = usePasskeys.getState().requestStepUp('Approve run 42')
    usePasskeys.getState().close()
    await expect(result).resolves.toBeNull()
  })

  it('refuses while the socket is down', async () => {
    usePasskeys.getState().openStepUp()
    await usePasskeys.getState().verify()
    expect(usePasskeys.getState().phase).toBe('refused')
  })
})
