import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { Cmd, Event, EventEnvelope } from '@/protocol/generated'
import type { CredentialLike } from '@/features/passkeys/codec'

const bus = vi.hoisted(() => ({
  sent: [] as Cmd[],
  listeners: [] as ((e: EventEnvelope) => void)[],
}))
vi.mock('@/app/connection', () => ({
  sendCmd: (cmd: Cmd) => {
    bus.sent.push(cmd)
    return bus.sent.length
  },
  onServerEvent: (l: (e: EventEnvelope) => void) => {
    bus.listeners.push(l)
    return () => undefined
  },
  onReconnected: () => () => undefined,
  connect: () => undefined,
  disconnect: () => undefined,
}))

const { assertionJson, attestationJson, creationOptions, fromB64Url, requestOptions, toB64Url } = await import(
  '@/features/passkeys/codec'
)
const { setAuthenticator, stepUpAssertion } = await import('@/features/passkeys/api')
const { usePasskeys, resetPasskeysStore } = await import('@/features/passkeys/store')

const bytes = (...b: number[]) => new Uint8Array(b).buffer

function emit(event: Event, reply_to?: number): void {
  for (const l of bus.listeners) l(reply_to === undefined ? { event } : { reply_to, event })
}

/** Lets the store's awaits run. */
const tick = () => new Promise((r) => setTimeout(r, 0))

describe('base64url', () => {
  it('round-trips empty input and every tail length, unpadded', () => {
    expect(toB64Url(new ArrayBuffer(0))).toBe('')
    expect(fromB64Url('').byteLength).toBe(0)
    for (const [raw, enc] of [
      [[0xfb], '-w'],
      [[0xfb, 0xff], '-_8'],
      [[0xfb, 0xff, 0xbf], '-_-_'],
    ] as const) {
      expect(toB64Url(bytes(...raw))).toBe(enc)
      expect(new Uint8Array(fromB64Url(enc))).toEqual(new Uint8Array(raw))
    }
  })

  it('round-trips 64 KiB', () => {
    const raw = new Uint8Array(64 * 1024)
    for (let i = 0; i < raw.length; i++) raw[i] = (i * 31 + 7) & 0xff
    const s = toB64Url(raw)
    expect(s).not.toMatch(/[=+/]/)
    expect(new Uint8Array(fromB64Url(s))).toEqual(raw)
  })

  it('refuses padding, other alphabets and impossible lengths', () => {
    expect(() => fromB64Url('AAE=')).toThrow()
    expect(() => fromB64Url('AA==')).toThrow()
    expect(() => fromB64Url('a+b/')).toThrow()
    expect(() => fromB64Url('AAAAA')).toThrow()
  })
})

describe('options from the protocol JSON', () => {
  const json = {
    rp: { id: 'depths.example.com', name: 'Depths' },
    user: { id: toB64Url(bytes(1, 2, 3)), name: 'ada', displayName: 'Ada' },
    challenge: toB64Url(bytes(9, 9)),
    pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
    excludeCredentials: [{ type: 'public-key', id: toB64Url(bytes(7)) }],
  }

  it('decodes a registration and keeps the rest', () => {
    const o = creationOptions(json)
    expect(new Uint8Array(o.challenge as ArrayBuffer)).toEqual(new Uint8Array([9, 9]))
    expect(new Uint8Array(o.user.id as ArrayBuffer)).toEqual(new Uint8Array([1, 2, 3]))
    expect(o.rp).toEqual({ id: 'depths.example.com', name: 'Depths' })
    expect(new Uint8Array(o.excludeCredentials?.[0]?.id as ArrayBuffer)).toEqual(new Uint8Array([7]))
  })

  it('unwraps webauthn-rs { publicKey } and refuses options without a challenge', () => {
    expect(creationOptions({ publicKey: json }).rp.name).toBe('Depths')
    expect(() => creationOptions({ ...json, challenge: undefined })).toThrow(/challenge/)
    expect(() => creationOptions({ ...json, challenge: 'AAE=' })).toThrow()
  })

  it('decodes an assertion request', () => {
    const o = requestOptions({
      publicKey: {
        challenge: toB64Url(bytes(5)),
        rpId: 'depths.example.com',
        allowCredentials: [{ type: 'public-key', id: toB64Url(bytes(4, 2)) }],
        userVerification: 'required',
      },
    })
    expect(new Uint8Array(o.challenge as ArrayBuffer)).toEqual(new Uint8Array([5]))
    expect(new Uint8Array(o.allowCredentials?.[0]?.id as ArrayBuffer)).toEqual(new Uint8Array([4, 2]))
    expect(o.userVerification).toBe('required')
  })
})

const attestation: CredentialLike = {
  rawId: bytes(1),
  response: { attestationObject: bytes(2), clientDataJSON: bytes(3), getTransports: () => ['internal'] },
  getClientExtensionResults: () => ({ credProps: { rk: true }, appid: true }),
}
const assertion: CredentialLike = {
  rawId: bytes(1),
  response: { authenticatorData: bytes(4), clientDataJSON: bytes(5), signature: bytes(6), userHandle: new ArrayBuffer(0) },
  getClientExtensionResults: () => ({}),
}
const assertionWire = {
  id: 'AQ',
  rawId: 'AQ',
  response: { authenticatorData: 'BA', clientDataJSON: 'BQ', signature: 'Bg' },
  type: 'public-key',
}

describe('credentials to the protocol JSON', () => {
  it('encodes an attestation with transports and only credProps', () => {
    expect(attestationJson(attestation)).toEqual({
      id: 'AQ',
      rawId: 'AQ',
      response: { attestationObject: 'Ag', clientDataJSON: 'Aw', transports: ['internal'] },
      type: 'public-key',
      extensions: { credProps: { rk: true } },
    })
  })

  it('encodes an assertion, leaving out an empty userHandle and absent extensions', () => {
    expect(assertionJson(assertion)).toEqual(assertionWire)
    const withHandle = { ...assertion, response: { ...(assertion.response as object), userHandle: bytes(8) } }
    expect(assertionJson(withHandle).response.userHandle).toBe('CA')
  })
})

const request = { ceremony: 'c-1', options: { challenge: toB64Url(bytes(1)) } }

describe('step-up assertion', () => {
  afterEach(() => setAuthenticator(undefined))

  it('signs the request it was given', async () => {
    setAuthenticator({ create: () => Promise.resolve(null), get: () => Promise.resolve(assertion) })
    expect(await stepUpAssertion(request)).toEqual({ ok: true, assertion: { ceremony: 'c-1', credential: assertionWire } })
  })

  it('reads a cancelled prompt as not confirmed', async () => {
    const notAllowed = Object.assign(new Error('The operation either timed out or was not allowed.'), { name: 'NotAllowedError' })
    setAuthenticator({ create: () => Promise.reject(notAllowed), get: () => Promise.reject(notAllowed) })
    expect(await stepUpAssertion(request)).toEqual({ ok: false, refusal: { kind: 'not_confirmed' } })
    setAuthenticator(null)
    expect(await stepUpAssertion(request)).toEqual({ ok: false, refusal: { kind: 'passkey', reason: 'unsupported' } })
  })
})

describe('passkeys store', () => {
  beforeEach(() => {
    bus.sent.length = 0
    resetPasskeysStore()
    setAuthenticator({ create: () => Promise.resolve(attestation), get: () => Promise.resolve(assertion) })
  })
  afterEach(() => setAuthenticator(undefined))

  it('follows every Verification, pushed or a reply', () => {
    const v = { verified: false, at: null, window_secs: 300, enrolled: true, required: true, recovering: false }
    emit({ type: 'Verification', data: v })
    expect(usePasskeys.getState().verification).toEqual(v)
    usePasskeys.getState().refreshVerification()
    expect(bus.sent.at(-1)).toEqual({ type: 'GetVerification' })
    emit({ type: 'Verification', data: { ...v, verified: true, at: '2026-10-28T12:00:00Z' } }, bus.sent.length)
    expect(usePasskeys.getState().verification?.verified).toBe(true)
  })

  it('verifies: begin, assert, finish', async () => {
    usePasskeys.getState().openVerify()
    const done = usePasskeys.getState().verify()
    expect(bus.sent.at(-1)).toEqual({ type: 'PasskeyBegin', data: { purpose: { kind: 'verify' } } })
    emit({ type: 'PasskeyChallenge', data: request }, bus.sent.length)
    await tick()
    expect(bus.sent.at(-1)).toEqual({ type: 'PasskeyFinish', data: { ceremony: 'c-1', credential: assertionWire } })
    emit({ type: 'Verification', data: { verified: true, at: null, window_secs: 300, enrolled: true, required: true, recovering: false } }, bus.sent.length)
    await done
    expect(usePasskeys.getState().phase).toBe('done')
  })

  it('registering on a device that already holds a passkey says so', async () => {
    const exists = Object.assign(new Error('The authenticator already holds a credential.'), { name: 'InvalidStateError' })
    setAuthenticator({ create: () => Promise.reject(exists), get: () => Promise.resolve(null) })
    usePasskeys.getState().openEnrol()
    const done = usePasskeys.getState().enrol('Phone')
    const options = {
      rp: { id: 'localhost', name: 'Depths' },
      user: { id: toB64Url(bytes(1)), name: 'ada', displayName: 'Ada' },
      challenge: toB64Url(bytes(2)),
      pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
    }
    emit({ type: 'PasskeyChallenge', data: { ceremony: 'c-1', options } }, bus.sent.length)
    await done
    expect(usePasskeys.getState().phase).toBe('refused')
    expect(usePasskeys.getState().error).toMatch(/already holds one of your passkeys/)
  })

  it('a cancelled prompt leaves the dialog open with the message', async () => {
    setAuthenticator({ create: () => Promise.resolve(null), get: () => Promise.resolve(null) })
    usePasskeys.getState().openVerify()
    const done = usePasskeys.getState().verify()
    emit({ type: 'PasskeyChallenge', data: request }, bus.sent.length)
    await done
    expect(usePasskeys.getState()).toMatchObject({ dialog: { kind: 'verify' }, phase: 'refused', error: 'The passkey prompt was cancelled.' })
  })

  it('requestStepUp answers the given request without PasskeyFinish, and null when closed', async () => {
    const asked = usePasskeys.getState().requestStepUp('Open a shell', request)
    await usePasskeys.getState().verify()
    expect(await asked).toEqual({ ceremony: 'c-1', credential: assertionWire })
    expect(bus.sent).toEqual([])
    expect(usePasskeys.getState().dialog).toBeNull()

    const closed = usePasskeys.getState().requestStepUp('Open a shell', request)
    usePasskeys.getState().close()
    expect(await closed).toBeNull()
  })

  it('requestFreshCheck finishes the ceremony, and a retry begins a fresh one', async () => {
    setAuthenticator({ create: () => Promise.resolve(null), get: () => Promise.resolve(null) })
    const asked = usePasskeys.getState().requestFreshCheck('Save the user', request)
    await usePasskeys.getState().verify()
    expect(usePasskeys.getState().phase).toBe('refused')

    setAuthenticator({ create: () => Promise.resolve(null), get: () => Promise.resolve(assertion) })
    const retry = usePasskeys.getState().verify()
    expect(bus.sent.at(-1)).toEqual({ type: 'PasskeyBegin', data: { purpose: { kind: 'verify' } } })
    emit({ type: 'PasskeyChallenge', data: { ...request, ceremony: 'c-2' } }, bus.sent.length)
    await tick()
    expect(bus.sent.at(-1)).toMatchObject({ type: 'PasskeyFinish', data: { ceremony: 'c-2' } })
    emit({ type: 'Verification', data: { verified: true, at: null, window_secs: 300, enrolled: true, required: false, recovering: false } }, bus.sent.length)
    await retry
    expect(await asked).toBe(true)
  })

  it('opens the enrol dialog for a plain user told to enrol, not over the admin banner', () => {
    emit({ type: 'PasskeyEnrolmentRequired' }, 99)
    expect(usePasskeys.getState().dialog).toEqual({ kind: 'enrol', required: true })
    usePasskeys.getState().close()
    emit({ type: 'Verification', data: { verified: false, at: null, window_secs: 300, enrolled: false, required: true, recovering: false } })
    emit({ type: 'PasskeyEnrolmentRequired' }, 100)
    expect(usePasskeys.getState().dialog).toBeNull()
  })
})
