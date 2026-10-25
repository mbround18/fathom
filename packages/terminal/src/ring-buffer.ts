/**
 * Fixed-capacity FIFO for high-volume streams such as pod logs (data-model.md §3 rule 4,
 * constitution V). Appending past capacity drops the oldest items and counts them in
 * `truncated`, so the UI can show an explicit truncation marker.
 *
 * The buffer is mutable for speed (a log burst may append thousands of lines per batch); the
 * store signals changes by replacing the owning `StreamState` object, not this buffer.
 */
export const DEFAULT_RING_CAPACITY = 10_000

export class RingBuffer<T> {
  readonly capacity: number
  /** Items dropped from the front since creation (or since `clear()`). */
  truncated = 0
  #items: (T | undefined)[]
  #start = 0
  #length = 0

  constructor(capacity: number = DEFAULT_RING_CAPACITY) {
    if (!Number.isInteger(capacity) || capacity < 1) {
      throw new RangeError(`RingBuffer capacity must be a positive integer, got ${capacity}`)
    }
    this.capacity = capacity
    this.#items = new Array<T | undefined>(capacity)
  }

  get length(): number {
    return this.#length
  }

  push(...items: T[]): void {
    this.pushAll(items)
  }

  pushAll(items: readonly T[]): void {
    // Only the last `capacity` items can survive; skip the rest without touching the array.
    const skip = Math.max(0, items.length - this.capacity)
    this.truncated += skip
    for (let i = skip; i < items.length; i++) {
      const end = (this.#start + this.#length) % this.capacity
      this.#items[end] = items[i]
      if (this.#length < this.capacity) {
        this.#length++
      } else {
        this.#start = (this.#start + 1) % this.capacity
        this.truncated++
      }
    }
  }

  /** Item `i` counted from the oldest retained item. */
  at(i: number): T | undefined {
    if (i < 0 || i >= this.#length) return undefined
    return this.#items[(this.#start + i) % this.capacity]
  }

  toArray(): T[] {
    const out: T[] = []
    for (let i = 0; i < this.#length; i++)
      out.push(this.#items[(this.#start + i) % this.capacity] as T)
    return out
  }

  clear(): void {
    this.#items = new Array<T | undefined>(this.capacity)
    this.#start = 0
    this.#length = 0
    this.truncated = 0
  }
}
