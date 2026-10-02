/**
 * Retries in the mock suite are only for browser-engine crashes, not for flaky tests.
 *
 * Playwright's WebKit build on Linux (WPE) sometimes segfaults in its compositor thread. The
 * kernel logs `ThreadedCompositor ... segfault at 0 ... in libWPEWebKit-2.0.so`, and it matches
 * WebKit bug 308242: "[GTK][WPE] AcceleratedSurface might segfault when createTarget() fails". The
 * web process dies mid-test. Playwright words that death several ways depending on which side
 * noticed first — "Target crashed", "Page crashed", "Target page, context or browser has been
 * closed", or WebKit's own "WebKit encountered an internal error" — and on a loaded machine the
 * launch itself can die instead, logging "D-Bus connection was disconnected" or "Received signal
 * 6". They are one failure with several spellings, so all of them belong in CRASH. A spelling
 * missing from it is worse than useless: the run is then forced to `failed` and the cause is
 * reported as a flaky test, which sends the next reader looking for a bug in the spec.
 *
 * This happens in roughly 1 of 2,000+ WebKit tests under a loaded machine, with the same crash
 * site every time, whatever the spec. Nothing in the app can prevent it.
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

// Every alternative here names the engine or its process dying. Nothing describing a *test*
// outcome — a timeout, a failed assertion, an unresolved locator — may be added: this regex is
// what buys a retry the right to be forgiven, so widening it to a symptom the app can cause
// would let a real regression pass by crashing first.
const CRASH =
  /Target crashed|Page crashed|Target page, context or browser has been closed|WebKit encountered an internal error|D-Bus connection was disconnected|Received signal 6/

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
