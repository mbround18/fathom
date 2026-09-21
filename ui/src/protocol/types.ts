/**
 * Protocol types for UI code. Everything comes from the ts-rs bindings in `./generated`
 * (never edited by hand); this module only adds the version-free aliases used across the UI.
 */
import type { BackendCmdV1, BackendEventEnvelopeV1, BackendEventV1 } from './generated'

export type * from './generated'

export type BackendCmd = BackendCmdV1
export type BackendEvent = BackendEventV1
export type BackendEventEnvelope = BackendEventEnvelopeV1

/** The only protocol version this UI understands (contracts/ipc.md). */
export const PROTOCOL_VERSION = 1
