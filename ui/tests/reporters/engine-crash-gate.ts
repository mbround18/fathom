/**
 * Retries in the mock suite are only for browser-engine crashes, not for flaky tests.
 *
 * Playwright's WebKit build on Linux (WPE) sometimes segfaults in its compositor thread. The
 * kernel logs `ThreadedCompositor ... segfault at 0 ... in libWPEWebKit-2.0.so`, and it matches
 * WebKit bug 308242: "[GTK][WPE] AcceleratedSurface might segfault when createTarget() fails". The
 * web process dies mid-test, and the test fails with "Target crashed" or "Target page, context or
 * browser has been closed". This happens in roughly 1 of 2,000+ WebKit tests under a loaded
 * machine, with the same crash site every time, whatever the spec. Nothing in the app can prevent
 * it.
 *
 * The WebKit projects therefore get one retry (playwright.config.ts). This reporter makes sure a
 * retry does not hide anything else: a test that passes only after a non-crash failure is still a
 * failure, and the run's status is overridden to `failed`.
 */
import type {
  FullResult,
  Reporter,
  TestCase,
  TestError,
  TestResult,
} from '@playwright/test/reporter'

const CRASH = /Target crashed|Target page, context or browser has been closed/

function stripAnsi(s: string): string {
  // eslint-disable-next-line no-control-regex
  return s.replace(/\u001b\[[0-9;]*m/g, '')
}

/** True when the attempt failed only because the browser's page process died under it. */
export function isEngineCrash(result: Pick<TestResult, 'status' | 'errors'>): boolean {
  if (result.status !== 'failed' || result.errors.length === 0) return false
  return result.errors.every((e: TestError) => CRASH.test(stripAnsi(e.message ?? '')))
}

export default class EngineCrashGate implements Reporter {
  readonly #crashRetried: string[] = []
  readonly #maskedFlakes: string[] = []

  printsToStdio(): boolean {
    return false
  }

  onTestEnd(test: TestCase, result: TestResult): void {
    if (result.status !== 'passed' || result.retry === 0) return
    const earlier = test.results.filter((r) => r !== result && r.status !== 'passed')
    const title = `[${test.parent.project()?.name ?? '?'}] ${test.titlePath().slice(2).join(' › ')}`
    if (earlier.length > 0 && earlier.every(isEngineCrash)) this.#crashRetried.push(title)
    else this.#maskedFlakes.push(title)
  }

  onEnd(result: FullResult): Promise<{ status?: FullResult['status'] } | undefined> {
    for (const t of this.#crashRetried) {
      console.log(`engine-crash-gate: retried after a browser crash (not a test flake): ${t}`)
    }
    if (this.#maskedFlakes.length === 0) return Promise.resolve(undefined)
    for (const t of this.#maskedFlakes) {
      console.log(`engine-crash-gate: FLAKY (failed without a browser crash, then passed): ${t}`)
    }
    return Promise.resolve(result.status === 'passed' ? { status: 'failed' } : undefined)
  }
}
