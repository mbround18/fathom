import { describe, expect, it } from 'vitest'

import { readCsrf } from '@/protocol/http'
import {
  assertionJson,
  attestationJson,
  creationOptions,
  fromB64Url,
  requestOptions,
  toB64Url,
  type CredentialLike,
} from '@/protocol/webauthn'

const bytes = (...b: number[]) => new Uint8Array(b).buffer

describe('base64url', () => {
  it('round-trips without padding and with the URL-safe alphabet', () => {
    const raw = bytes(0xfb, 0xff, 0xbf, 0x00, 0x01)
    const s = toB64Url(raw)
    expect(s).toBe('-_-_AAE')
    expect(new Uint8Array(fromB64Url(s))).toEqual(new Uint8Array(raw))
  })

  it('accepts padding and refuses other alphabets', () => {
    expect(new Uint8Array(fromB64Url('AAE='))).toEqual(new Uint8Array([0, 1]))
    expect(() => fromB64Url('a+b/')).toThrow()
  })
})

describe('creationOptions', () => {
  const json = {
    rp: { id: 'keel.example.com', name: 'Keel' },
    user: { id: toB64Url(bytes(1, 2, 3)), name: 'ada', displayName: 'Ada' },
    challenge: toB64Url(bytes(9, 9)),
    pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
    excludeCredentials: [{ type: 'public-key', id: toB64Url(bytes(7)) }],
  }

  it('decodes the binary fields and keeps the rest', () => {
    const o = creationOptions(json)
    expect(new Uint8Array(o.challenge as ArrayBuffer)).toEqual(new Uint8Array([9, 9]))
    expect(new Uint8Array(o.user.id as ArrayBuffer)).toEqual(new Uint8Array([1, 2, 3]))
    expect(o.user.name).toBe('ada')
    expect(o.rp).toEqual({ id: 'keel.example.com', name: 'Keel' })
    expect(new Uint8Array(o.excludeCredentials?.[0]?.id as ArrayBuffer)).toEqual(new Uint8Array([7]))
  })

  it('unwraps webauthn-rs { publicKey } and refuses options without a challenge', () => {
    expect(creationOptions({ publicKey: json }).rp.name).toBe('Keel')
    expect(() => creationOptions({ ...json, challenge: undefined })).toThrow(/challenge/)
  })
})

describe('requestOptions', () => {
  it('decodes the challenge and allowCredentials', () => {
    const o = requestOptions({
      challenge: toB64Url(bytes(5)),
      rpId: 'keel.example.com',
      allowCredentials: [{ type: 'public-key', id: toB64Url(bytes(4, 2)) }],
      userVerification: 'required',
    })
    expect(new Uint8Array(o.challenge as ArrayBuffer)).toEqual(new Uint8Array([5]))
    expect(new Uint8Array(o.allowCredentials?.[0]?.id as ArrayBuffer)).toEqual(new Uint8Array([4, 2]))
    expect(o.userVerification).toBe('required')
  })
})

describe('credential JSON', () => {
  it('encodes an attestation with transports and only credProps', () => {
    const c: CredentialLike = {
      rawId: bytes(1),
      response: {
        attestationObject: bytes(2),
        clientDataJSON: bytes(3),
        getTransports: () => ['internal'],
      },
      getClientExtensionResults: () => ({ credProps: { rk: true }, appid: true }),
    }
    expect(attestationJson(c)).toEqual({
      id: 'AQ',
      rawId: 'AQ',
      response: { attestationObject: 'Ag', clientDataJSON: 'Aw', transports: ['internal'] },
      type: 'public-key',
      extensions: { credProps: { rk: true } },
    })
  })

  it('encodes an assertion, leaving out an empty userHandle and absent extensions', () => {
    const c: CredentialLike = {
      rawId: bytes(1),
      response: {
        authenticatorData: bytes(4),
        clientDataJSON: bytes(5),
        signature: bytes(6),
        userHandle: new ArrayBuffer(0),
      },
      getClientExtensionResults: () => ({}),
    }
    expect(assertionJson(c)).toEqual({
      id: 'AQ',
      rawId: 'AQ',
      response: { authenticatorData: 'BA', clientDataJSON: 'BQ', signature: 'Bg' },
      type: 'public-key',
    })
  })
})

describe('readCsrf', () => {
  it('prefers the __Host- cookie, falls back to the dev name, and ignores junk', () => {
    expect(readCsrf('a=1; keel_csrf=dev-tok; __Host-keel_csrf=sec_tok')).toBe('sec_tok')
    expect(readCsrf('keel_csrf=dev-tok')).toBe('dev-tok')
    expect(readCsrf('keel_csrf=')).toBeNull()
    expect(readCsrf('keel_csrf=<script>')).toBeNull()
    expect(readCsrf('')).toBeNull()
  })
})
