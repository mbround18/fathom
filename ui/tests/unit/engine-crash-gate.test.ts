/**
 * The crash gate decides whether a retry is forgiven or forced to fail the run, so the set of
 * messages it recognises is load-bearing in both directions. These cases are the real messages
 * observed on this machine, kept verbatim.
 */
import { describe, expect, it } from 'vitest'

import { isEngineCrash } from '../reporters/engine-crash-gate'

const failed = (...messages: string[]) =>
  ({ status: 'failed', errors: messages.map((message) => ({ message })) }) as const

describe('isEngineCrash: the engine died', () => {
  // Each of these was misreported as a flaky test before it was recognised, which forced an
  // otherwise-green run to fail and pointed the reader at the spec instead of the machine.
  it.each([
    ['Target crashed', 'Error: page.evaluate: Target crashed'],
    ['Page crashed', 'Error: page.goto: Page crashed'],
    ['a closed context', 'Target page, context or browser has been closed'],
    ['WebKit internal error', 'WebKit encountered an internal error'],
    ['a dead D-Bus', 'FATAL:dbus/bus.cc(406) D-Bus connection was disconnected'],
    ['an aborted launch', 'browserType.launch: Received signal 6'],
  ])('is forgiven: %s', (_name, message) => {
    expect(isEngineCrash(failed(message))).toBe(true)
  })
})

describe('isEngineCrash: the test failed', () => {
  // The inverse half is the one that protects the suite: a real failure must never buy a retry.
  it.each([
    ['a timeout', 'Test timeout of 120000ms exceeded.'],
    ['a missing element', 'expect(locator).toBeVisible() failed\nError: element(s) not found'],
    ['a wrong attribute', 'expect(locator).toHaveAttribute(expected) failed'],
  ])('is not forgiven: %s', (_name, message) => {
    expect(isEngineCrash(failed(message))).toBe(false)
  })

  it('is not forgiven when a crash is only one of several errors', () => {
    // `every` is deliberate: an attempt that also failed an assertion is a test failure that
    // happened to crash afterwards, not a crash.
    expect(isEngineCrash(failed('Target crashed', 'expect(received).toBe(expected)'))).toBe(false)
  })

  it('is not forgiven when the attempt passed, or failed with no error at all', () => {
    expect(isEngineCrash({ status: 'passed', errors: [{ message: 'Target crashed' }] })).toBe(false)
    expect(isEngineCrash({ status: 'failed', errors: [] })).toBe(false)
  })
})
