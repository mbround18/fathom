/**
 * WebAuthn in the browser, for the passkey dialogs (spec 055 T030, the 054 ceremony).
 *
 * The server sends `PasskeyRequest.options`, the `publicKey` options in WebAuthn's JSON form
 * (binary fields base64url; webauthn-rs may wrap them in `{ publicKey }`). This module turns them
 * into the `ArrayBuffer` form `navigator.credentials` takes, and the credential it returns into the
 * JSON the server expects (`AttestationCredential`, `AssertionCredential`), carrying only the
 * `credProps` extension result: the server refuses unknown fields.
 *
 * Done by hand rather than with `PublicKeyCredential.parse*OptionsFromJSON` so it works the same
 * in every supported browser and in unit tests.
 */
import type { AssertionCredential, AttestationCredential, PasskeyExtensions } from './wire'

/** Unpadded base64url of `bytes`. */
export function toB64Url(bytes: ArrayBuffer | ArrayBufferView): string {
  const view =
    bytes instanceof ArrayBuffer ? new Uint8Array(bytes) : new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  let bin = ''
  for (const b of view) bin += String.fromCharCode(b)
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

/** Bytes of base64url (padded or not). Throws on anything that is not base64url. */
export function fromB64Url(s: string): ArrayBuffer {
  if (!/^[A-Za-z0-9_-]*={0,2}$/.test(s)) throw new Error('not base64url')
  const b64 = s.replace(/=+$/, '').replace(/-/g, '+').replace(/_/g, '/')
  const bin = atob(b64 + '='.repeat((4 - (b64.length % 4)) % 4))
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out.buffer
}

type Json = Record<string, unknown>

function isObject(v: unknown): v is Json {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

/** The `publicKey` object, unwrapped from `{ publicKey }` when the server sent it that way. */
function publicKeyOf(options: unknown): Json {
  if (!isObject(options)) throw new Error('passkey options are not an object')
  const inner = options.publicKey
  return isObject(inner) ? inner : options
}

function b64Field(o: Json, key: string): ArrayBuffer {
  const v = o[key]
  if (typeof v !== 'string') throw new Error(`passkey options: ${key} is missing`)
  return fromB64Url(v)
}

function descriptors(v: unknown): PublicKeyCredentialDescriptor[] | undefined {
  if (!Array.isArray(v)) return undefined
  return v.filter(isObject).map((d) => ({
    ...(d as unknown as PublicKeyCredentialDescriptor),
    type: 'public-key',
    id: b64Field(d, 'id'),
  }))
}

/** `PasskeyRequest.options` for a registration, as `navigator.credentials.create` takes them. */
export function creationOptions(options: unknown): PublicKeyCredentialCreationOptions {
  const pk = publicKeyOf(options)
  const user = pk.user
  if (!isObject(user)) throw new Error('passkey options: user is missing')
  const exclude = descriptors(pk.excludeCredentials)
  return {
    ...(pk as unknown as PublicKeyCredentialCreationOptions),
    challenge: b64Field(pk, 'challenge'),
    user: { ...(user as unknown as PublicKeyCredentialUserEntity), id: b64Field(user, 'id') },
    ...(exclude ? { excludeCredentials: exclude } : {}),
  }
}

/** `PasskeyRequest.options` for an assertion, as `navigator.credentials.get` takes them. */
export function requestOptions(options: unknown): PublicKeyCredentialRequestOptions {
  const pk = publicKeyOf(options)
  const allow = descriptors(pk.allowCredentials)
  return {
    ...(pk as unknown as PublicKeyCredentialRequestOptions),
    challenge: b64Field(pk, 'challenge'),
    ...(allow ? { allowCredentials: allow } : {}),
  }
}

/** The structural part of a `PublicKeyCredential` this module reads, so tests can build one. */
export interface CredentialLike {
  rawId: ArrayBuffer
  response: unknown
  getClientExtensionResults?: () => AuthenticationExtensionsClientOutputs
}

function extensionsOf(c: CredentialLike): PasskeyExtensions | undefined {
  const props = c.getClientExtensionResults?.().credProps
  if (!props) return undefined
  return { credProps: typeof props.rk === 'boolean' ? { rk: props.rk } : {} }
}

/** A registration's credential, as `PasskeyFinish` carries it. */
export function attestationJson(c: CredentialLike): AttestationCredential {
  const r = c.response as AuthenticatorAttestationResponse
  const id = toB64Url(c.rawId)
  const transports = typeof r.getTransports === 'function' ? r.getTransports() : []
  const extensions = extensionsOf(c)
  return {
    id,
    rawId: id,
    response: {
      attestationObject: toB64Url(r.attestationObject),
      clientDataJSON: toB64Url(r.clientDataJSON),
      transports,
    },
    type: 'public-key',
    ...(extensions ? { extensions } : {}),
  }
}

/** An assertion's credential, as `PasskeyFinish` or `Confirm.step_up` carries it. */
export function assertionJson(c: CredentialLike): AssertionCredential {
  const r = c.response as AuthenticatorAssertionResponse
  const id = toB64Url(c.rawId)
  const extensions = extensionsOf(c)
  return {
    id,
    rawId: id,
    response: {
      authenticatorData: toB64Url(r.authenticatorData),
      clientDataJSON: toB64Url(r.clientDataJSON),
      signature: toB64Url(r.signature),
      ...(r.userHandle && r.userHandle.byteLength > 0 ? { userHandle: toB64Url(r.userHandle) } : {}),
    },
    type: 'public-key',
    ...(extensions ? { extensions } : {}),
  }
}

/** What the browser can do; `navigator.credentials` in production, a stub in unit tests. */
export interface Authenticator {
  create(options: PublicKeyCredentialCreationOptions): Promise<CredentialLike | null>
  get(options: PublicKeyCredentialRequestOptions): Promise<CredentialLike | null>
}

export function browserAuthenticator(): Authenticator | null {
  const creds = (globalThis.navigator as Navigator | undefined)?.credentials
  if (!creds || typeof globalThis.PublicKeyCredential === 'undefined') return null
  return {
    create: async (publicKey) => (await creds.create({ publicKey })) as unknown as CredentialLike | null,
    get: async (publicKey) => (await creds.get({ publicKey })) as unknown as CredentialLike | null,
  }
}
