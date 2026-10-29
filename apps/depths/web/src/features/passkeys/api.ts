/**
 * The browser's authenticator (spec 054 T065): `navigator.credentials.create` and `.get` over the
 * server's options, returning the credential JSON the protocol carries.
 *
 * Every way the prompt can fail to complete (the person cancelled, it timed out, no authenticator)
 * ends in a {@link PasskeyFailure}: `cancelled` for the prompt, `unsupported` for a browser
 * without WebAuthn. Unit tests swap the authenticator with {@link setAuthenticator}.
 */
import type { AssertionCredential, AttestationCredential, PasskeyAssertion, PasskeyRequest } from '@/protocol/generated'

import {
  assertionJson,
  attestationJson,
  creationOptions,
  requestOptions,
  type CredentialLike,
} from './codec'
import type { StepUpRefusal } from './messages'

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

let override: Authenticator | null | undefined
/** For tests: the authenticator the ceremonies use (`undefined` = `navigator.credentials`). */
export function setAuthenticator(a: Authenticator | null | undefined): void {
  override = a
}
function current(): Authenticator | null {
  return override === undefined ? browserAuthenticator() : override
}

/** Why the authenticator did not answer. */
export class PasskeyFailure extends Error {
  readonly reason: 'cancelled' | 'unsupported' | 'invalid' | 'exists'
  constructor(reason: 'cancelled' | 'unsupported' | 'invalid' | 'exists') {
    super(reason)
    this.reason = reason
  }
}

async function run<T>(body: (a: Authenticator) => Promise<CredentialLike | null>, encode: (c: CredentialLike) => T): Promise<T> {
  const auth = current()
  if (!auth) throw new PasskeyFailure('unsupported')
  let c: CredentialLike | null
  try {
    c = await body(auth)
  } catch (e) {
    // Options the codec cannot read are the server's fault, not the person's.
    if (e instanceof PasskeyFailure) throw e
    if (e instanceof Error && /passkey options|base64url/.test(e.message)) throw new PasskeyFailure('invalid')
    // Registering on a device that already holds one of the person's passkeys.
    if (e instanceof Error && e.name === 'InvalidStateError') throw new PasskeyFailure('exists')
    // NotAllowedError (cancelled or timed out), AbortError and the like.
    throw new PasskeyFailure('cancelled')
  }
  if (!c) throw new PasskeyFailure('cancelled')
  return encode(c)
}

/** Registers a passkey over a `register` ceremony's options. */
export function createCredential(options: unknown): Promise<AttestationCredential> {
  return run((a) => a.create(creationOptions(options)), attestationJson)
}

/** Asserts over a `verify` ceremony's options. */
export function getAssertion(options: unknown): Promise<AssertionCredential> {
  return run((a) => a.get(requestOptions(options)), assertionJson)
}

/** Answers a step-up request, for `Confirm.assertion` or `OpenShell.assertion`. */
export async function signRequest(request: PasskeyRequest): Promise<PasskeyAssertion> {
  return { ceremony: request.ceremony, credential: await getAssertion(request.options) }
}

/**
 * A step-up inside a confirm (T073): the authenticator asserts over `request` and the result rides
 * on the command. A prompt that did not complete comes back as the refusal the flow shows.
 */
export async function stepUpAssertion(
  request: PasskeyRequest,
): Promise<{ ok: true; assertion: PasskeyAssertion } | { ok: false; refusal: StepUpRefusal }> {
  try {
    return { ok: true, assertion: await signRequest(request) }
  } catch (e) {
    if (e instanceof PasskeyFailure && e.reason !== 'cancelled')
      return { ok: false, refusal: { kind: 'passkey', reason: e.reason === 'exists' ? 'invalid' : e.reason } }
    return { ok: false, refusal: { kind: 'not_confirmed' } }
  }
}
