/**
 * Why a load or a change didn't happen, as the UI shows it (spec 065 R3). This is a plain UI shape,
 * not a protocol type: each app maps its own denials and errors to it, and owns its `OFFLINE` value
 * and the wording of it.
 */

/** A refusal as the server sent it, or the app's `OFFLINE`. */
export type Refusal =
  | { kind: 'denied'; action: string; reason: string }
  | { kind: 'error'; code: string; message: string }

/** A failure on the app's side of the connection rather than a refusal from the server. */
export function isTransportFailure(r: Refusal): boolean {
  return r.kind === 'error' && r.code === 'offline'
}

/** Where a list read lands: its value, or why it failed. */
export interface ListHandlers<T> {
  loaded: (value: T) => void
  failed: (refusal: Refusal) => void
}

/** A list's load, for a store that keeps it as one field. */
export type ListLoad<T> =
  | { status: 'loading' }
  | { status: 'loaded'; value: T }
  | { status: 'failed'; refusal: Refusal }
