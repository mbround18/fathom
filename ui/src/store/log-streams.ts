/**
 * Client-chosen pod log stream ids (feedback #2). Every logs view starts its own backend stream
 * under an id it picks here, so views never pick up or stop each other's streams.
 *
 * Ids start at `CLIENT_STREAM_ID_MIN` (2^32, mirroring `src/protocol/v1/ids.rs`): below it are
 * the backend's own counter ids (exec sessions), so the two never collide. The rest of the id is
 * random rather than a counter because a hosted backend is shared by every tab its user has
 * open, and two tabs counting from the same base would collide.
 */

/** Mirrors `CLIENT_STREAM_ID_MIN` in `src/protocol/v1/ids.rs`. */
export const CLIENT_STREAM_ID_MIN = 2 ** 32

/** Whether `id` is a client-chosen (log) stream id rather than a backend-allocated one. */
export function isClientStreamId(id: number): boolean {
  return id >= CLIENT_STREAM_ID_MIN
}

/** A fresh random stream id in `[2^32, 2^53)`, so it stays exact as a JS number. */
export function newLogStreamId(): number {
  const words = new Uint32Array(2)
  crypto.getRandomValues(words)
  const high = (words[0] ?? 0) & 0x1fffff // 21 bits: 2^21 * 2^32 = 2^53
  const id = high * 2 ** 32 + (words[1] ?? 0)
  return id < CLIENT_STREAM_ID_MIN ? id + CLIENT_STREAM_ID_MIN : id
}
