/**
 * Copy-on-write view of the store used while reducing one batch, so `ingest` can apply many
 * envelopes and still commit a single `set()`. Slices are shallow-copied on first write; slices
 * that no envelope touched keep their identity (and their subscribers stay quiet).
 */
import type { StoreData } from './index'

export type Effect = () => void

export class Draft {
  readonly #base: StoreData
  readonly #out: Partial<StoreData> = {}
  /** Side effects (toasts) to run after the batch is committed. */
  readonly effects: Effect[] = []

  constructor(base: StoreData) {
    this.#base = base
  }

  /** Current value of a slice (edited or not). Do not mutate the result. */
  read<K extends keyof StoreData>(key: K): StoreData[K] {
    return this.#out[key] ?? this.#base[key]
  }

  /** A writable shallow copy of a slice, created on first use within this batch. */
  edit<K extends keyof StoreData>(key: K): StoreData[K] {
    let slice = this.#out[key]
    if (slice === undefined) {
      slice = { ...this.#base[key] }
      this.#out[key] = slice
    }
    return slice as StoreData[K]
  }

  /** Replace a slice wholesale. */
  put<K extends keyof StoreData>(key: K, value: StoreData[K]): void {
    this.#out[key] = value
  }

  get patch(): Partial<StoreData> {
    return this.#out
  }
}
