import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { LinterPanel } from '@/components/panels/LinterPanel'
import {
  beginRefresh,
  clearLintCache,
  displayedReport,
  formatAge,
  isFresh,
  LINT_CACHE_TTL_MS,
  lintCacheEntry,
  lintScopeKey,
  needsRefresh,
  recordReport,
} from '@/linter/cache'
import type { LintReport } from '@/linter/types'
import { SECTIONS } from '@/linter/util'
import { descriptorFor } from '@/resources'
import { listKey, useStore } from '@/store'

function report(pendingObjects: number, score: number | null = 100): LintReport {
  return { findings: [], rules: [], counts: { high: 0, medium: 0, low: 0 }, score, pendingObjects }
}

describe('linter cache bookkeeping (#24)', () => {
  beforeEach(() => clearLintCache())

  it('keys entries per context and namespace and keeps them across lookups', () => {
    const a = lintCacheEntry(lintScopeKey('prod', null))
    expect(lintCacheEntry(lintScopeKey('prod', null))).toBe(a)
    expect(lintCacheEntry(lintScopeKey('dev', null))).not.toBe(a)
    expect(lintCacheEntry(lintScopeKey('prod', 'default'))).not.toBe(a)
  })

  it('refreshes on first use and after the TTL, reusing a run started within it', () => {
    const e = lintCacheEntry('k')
    expect(needsRefresh(e, 1_000)).toBe(true)
    beginRefresh(e, 1_000, false)
    // Still in flight or finished: either way a remount reuses it.
    expect(needsRefresh(e, 1_000 + LINT_CACHE_TTL_MS)).toBe(false)
    expect(needsRefresh(e, 1_001 + LINT_CACHE_TTL_MS)).toBe(true)
  })

  it('a plain refresh keeps sent reads; a manual one forgets them and stales details', () => {
    const e = lintCacheEntry('k')
    e.sent.set('Pod|a|p', 5)
    beginRefresh(e, 10, false)
    expect(e.sent.size).toBe(1)
    expect(isFresh(e.detailsSince, 3)).toBe(true)
    beginRefresh(e, 20, true)
    expect(e.sent.size).toBe(0)
    expect(isFresh(e.detailsSince, 19)).toBe(false)
    expect(isFresh(e.detailsSince, 20)).toBe(true)
    expect(isFresh(e.detailsSince, undefined)).toBe(false)
  })

  it('records only complete reports and shows the cached one while a run is partial', () => {
    const e = lintCacheEntry('k')
    const partial = report(3)
    // No cache yet: the partial report streams in as before.
    expect(recordReport(e, partial, true, 1)).toBe(false)
    expect(displayedReport(e, partial, true)).toBe(partial)

    const done = report(0, 80)
    expect(recordReport(e, done, false, 2)).toBe(false) // lists still loading
    expect(recordReport(e, done, true, 3)).toBe(true)
    expect(e.evaluatedAt).toBe(3)

    const rerun = report(12)
    expect(displayedReport(e, rerun, true)).toBe(done)
    expect(displayedReport(e, report(0), false)).toBe(done)
    const fresh = report(0, 90)
    expect(displayedReport(e, fresh, true)).toBe(fresh)
  })

  it('formats ages', () => {
    expect(formatAge(0)).toBe('just now')
    expect(formatAge(42_000)).toBe('42s ago')
    expect(formatAge(3 * 60_000)).toBe('3m ago')
    expect(formatAge(2 * 3_600_000)).toBe('2h ago')
  })
})

describe('LinterPanel reuses its run across mounts (#24)', () => {
  const refreshSection = vi.fn()
  const inspect = vi.fn()

  beforeEach(() => {
    clearLintCache()
    refreshSection.mockReset()
    inspect.mockReset()
    useStore.setState({
      send: () => undefined,
      refreshSection,
      inspect,
      lists: {},
      details: {},
    } as never)
    useStore.setState((s) => ({
      session: {
        ...s.session,
        contextsLoaded: true,
        contexts: [{ name: 'prod' }] as never,
        activeContext: 'prod',
        activeNamespace: null,
      },
    }))
  })
  afterEach(() => cleanup())

  it('refreshes lists on first mount only, and again on the Refresh button', () => {
    const first = render(<LinterPanel paneId="p1" />)
    expect(refreshSection).toHaveBeenCalled()
    const perRun = refreshSection.mock.calls.length

    // The lists arrive and settle (the mock never marks them loading).
    act(() => {
      useStore.setState({
        lists: Object.fromEntries(
          Object.values(SECTIONS).map((section) => [
            listKey(
              'prod',
              section,
              descriptorFor(section)?.namespaced === false ? 'Cluster' : 'Namespaced',
              null,
            ),
            { rows: [], status: 'ready', updatedAt: Date.now() },
          ]),
        ),
      } as never)
    })
    first.unmount()

    render(<LinterPanel paneId="p1" />)
    expect(refreshSection).toHaveBeenCalledTimes(perRun)
    expect(screen.getByTestId('linter-age').textContent).toMatch(/Updated just now/)

    fireEvent.click(screen.getByTestId('linter-refresh'))
    expect(refreshSection).toHaveBeenCalledTimes(perRun * 2)
  })

  it('refreshes again on mount once the cached run is older than the TTL', () => {
    const t0 = Date.now()
    const now = vi.spyOn(Date, 'now').mockReturnValue(t0)
    const first = render(<LinterPanel paneId="p1" />)
    const perRun = refreshSection.mock.calls.length
    first.unmount()

    now.mockReturnValue(t0 + LINT_CACHE_TTL_MS + 1)
    render(<LinterPanel paneId="p1" />)
    expect(refreshSection).toHaveBeenCalledTimes(perRun * 2)
  })

  it('a different context gets its own run', () => {
    const first = render(<LinterPanel paneId="p1" />)
    const perRun = refreshSection.mock.calls.length
    first.unmount()
    useStore.setState((s) => ({ session: { ...s.session, activeContext: 'dev' } }))
    render(<LinterPanel paneId="p1" />)
    expect(refreshSection).toHaveBeenCalledTimes(perRun * 2)
  })
})
