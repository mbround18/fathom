/**
 * PROVISIONAL passkey wire types (spec 055 T030), pending T028.
 *
 * The passkey commands are being added to `crates/keel-protocol` in parallel (T028) and are not in
 * `./generated` yet. Until they are, this file declares them with the exact shapes Depths
 * generates (apps/depths/web/src/protocol/generated, spec 054), and widens the generated `Cmd`,
 * `Event` and their envelopes to include them.
 *
 * When T028 merges and the bindings are regenerated, replace this whole file with re-exports from
 * `./generated` (or delete it and drop its line in `./wire.ts`). Nothing else imports it: the app
 * imports protocol types from `@/protocol/wire` only.
 */
import type {
  AssertionCredential,
  B64Url,
  CeremonyId,
  Cmd as GeneratedCmd,
  Event as GeneratedEvent,
  PasskeyExtensions,
  PublicKeyType,
  RequestId,
  Verification,
} from './generated'

/** A WebAuthn transport hint (`usb`, `nfc`, `internal`, `hybrid`, ...). Hints only. */
export type PasskeyTransport = string

/** `response` of a registration (`AuthenticatorAttestationResponse`), binary fields base64url. */
export type AttestationResponse = {
  attestationObject: B64Url
  clientDataJSON: B64Url
  transports: Array<PasskeyTransport>
}

/** A `PublicKeyCredential` from `navigator.credentials.create`. */
export type AttestationCredential = {
  id: B64Url
  rawId: B64Url
  response: AttestationResponse
  /** Always `public-key`. */
  type: PublicKeyType
  extensions?: PasskeyExtensions
}

/** A browser's answer to a `PasskeyRequest`, told apart by its `response` fields. */
export type PasskeyCredential = AttestationCredential | AssertionCredential

/** A person's name for one of their passkeys: 1–64 characters, no surrounding spaces. */
export type PasskeyName = string

/** What a passkey ceremony is for. */
export type PasskeyPurpose = { kind: 'register'; name: PasskeyName } | { kind: 'verify' }

/**
 * A ceremony the browser must answer with `navigator.credentials.create` (registration) or
 * `.get` (assertion). `options` is the server's `publicKey` options in WebAuthn's JSON form.
 */
export type PasskeyRequest = { ceremony: CeremonyId; options: unknown }

/** The answer to a ceremony. */
export type PasskeyResponse = { ceremony: CeremonyId; credential: PasskeyCredential }

/** One of the person's own passkeys. The credential itself never leaves the server. */
export type PasskeyInfo = {
  /** The credential id, base64url. */
  id: string
  name: string
  /** RFC 3339 UTC. */
  created: string
  last_used: string | null
}

/** Why a passkey answer was refused. Audited as `passkey_failed`. */
export type PasskeyRefusedReason =
  | 'replayed'
  | 'wrong_origin'
  | 'unknown_credential'
  | 'expired'
  | 'cancelled'
  | 'recovery_invalid'
  | 'invalid'
  | 'last_passkey'

/** The passkey commands, as Depths' `Cmd` has them. */
export type PasskeyCmd =
  | { type: 'PasskeyBegin'; data: { purpose: PasskeyPurpose } }
  | { type: 'PasskeyFinish'; data: PasskeyResponse }
  | { type: 'ListPasskeys' }
  /**
   * `Preview` (with `step_up` when the session's check is stale); the generated `Confirm` removes
   * it (`Confirmed`). `PasskeyRefused {last_passkey}` for the person's last passkey.
   */
  | { type: 'PreviewPasskeyRemove'; data: { id: B64Url } }

/** The passkey events, as Depths' `Event` has them. */
export type PasskeyEvent =
  | { type: 'PasskeyEnrolmentRequired' }
  | { type: 'PasskeyRefused'; data: { reason: PasskeyRefusedReason } }
  | { type: 'PasskeyChallenge'; data: PasskeyRequest }
  | { type: 'PasskeyRegistered'; data: PasskeyInfo }
  | { type: 'Passkeys'; data: { items: Array<PasskeyInfo> } }
  | { type: 'Verification'; data: Verification }

export type Cmd = GeneratedCmd | PasskeyCmd
export type Event = GeneratedEvent | PasskeyEvent

export type CmdEnvelope = { id: RequestId; cmd: Cmd }
export type EventEnvelope = {
  /** The `CmdEnvelope.id` this answers; absent for pushed events. */
  reply_to?: RequestId
  event: Event
}
