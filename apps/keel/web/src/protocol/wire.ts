/**
 * The protocol types the app uses: the generated bindings, with `Cmd`, `Event` and their
 * envelopes widened by the provisional passkey types until T028 lands (see ./passkey-pending.ts).
 * Explicit re-exports take precedence over `export *`, so the widened names win.
 */
export * from './generated'
export type {
  AttestationCredential,
  AttestationResponse,
  Cmd,
  CmdEnvelope,
  Event,
  EventEnvelope,
  PasskeyCmd,
  PasskeyCredential,
  PasskeyEvent,
  PasskeyInfo,
  PasskeyName,
  PasskeyPurpose,
  PasskeyRefusedReason,
  PasskeyRequest,
  PasskeyResponse,
  PasskeyTransport,
} from './passkey-pending'
