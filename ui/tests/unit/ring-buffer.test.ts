import { describe, expect, it } from 'vitest'

import { DEFAULT_RING_CAPACITY, RingBuffer } from '@/store/ring-buffer'

describe('RingBuffer', () => {
  it('defaults to a 10,000-item capacity', () => {
    expect(new RingBuffer<string>().capacity).toBe(10_000)
    expect(DEFAULT_RING_CAPACITY).toBe(10_000)
  })

  it('keeps items in insertion order below capacity', () => {
    const buf = new RingBuffer<number>(5)
    buf.push(1, 2, 3)
    expect(buf.length).toBe(3)
    expect(buf.toArray()).toEqual([1, 2, 3])
    expect(buf.truncated).toBe(0)
  })

  it('drops the oldest items past capacity and counts them', () => {
    const buf = new RingBuffer<number>(3)
    buf.push(1, 2, 3, 4)
    buf.push(5)
    expect(buf.toArray()).toEqual([3, 4, 5])
    expect(buf.length).toBe(3)
    expect(buf.truncated).toBe(2)
  })

  it('handles a single append larger than capacity', () => {
    const buf = new RingBuffer<number>(3)
    buf.push(0)
    buf.pushAll([1, 2, 3, 4, 5, 6])
    expect(buf.toArray()).toEqual([4, 5, 6])
    expect(buf.truncated).toBe(4)
  })

  it('bounds a 25,000-line log burst at 10,000 lines', () => {
    const buf = new RingBuffer<string>()
    for (let chunk = 0; chunk < 25; chunk++) {
      buf.pushAll(Array.from({ length: 1000 }, (_, i) => `line ${chunk * 1000 + i}`))
    }
    expect(buf.length).toBe(10_000)
    expect(buf.truncated).toBe(15_000)
    expect(buf.at(0)).toBe('line 15000')
    expect(buf.at(9_999)).toBe('line 24999')
  })

  it('at() is bounds-checked and clear() resets everything', () => {
    const buf = new RingBuffer<number>(2)
    buf.push(1, 2, 3)
    expect(buf.at(-1)).toBeUndefined()
    expect(buf.at(2)).toBeUndefined()
    buf.clear()
    expect(buf.length).toBe(0)
    expect(buf.truncated).toBe(0)
    expect(buf.toArray()).toEqual([])
  })

  it('rejects a non-positive capacity', () => {
    expect(() => new RingBuffer(0)).toThrow(RangeError)
  })
})
