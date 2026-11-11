/**
 * Spec 066 T030 (R1): the current workspace. `?ws=<slug>` first, then `localStorage["keel.workspace"]`,
 * then the first visible workspace. Archived and vanished ones are skipped, and the address and the
 * stored choice follow whatever was resolved, replacing the history entry rather than pushing one.
 */
import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import {
  resolveWorkspace,
  STORAGE_KEY,
  useCurrentWorkspace,
  useWorkspaceStore,
  useWorkspaceSync,
  visibleWorkspaces,
  withWorkspace,
} from '@/app/workspace-store'
import { useGovernance } from '@/features/teams/store'
import type { WorkspaceInfo } from '@/protocol/wire'

function ws(slug: string, over: Partial<WorkspaceInfo> = {}): WorkspaceInfo {
  return {
    id: `id-${slug}`,
    slug,
    name: slug.toUpperCase(),
    created: '2026-11-01T00:00:00Z',
    archived: false,
    projects: [],
    system: false,
    primary_binding: null,
    ...over,
  }
}

const OPS = ws('ops')
const PAYMENTS = ws('payments')
const ARCHIVED = ws('attic', { archived: true })
const SYSTEM = ws('a-system', { system: true })

describe('visibleWorkspaces', () => {
  it('drops archived ones and orders by slug, the System workspace last', () => {
    expect(visibleWorkspaces([PAYMENTS, SYSTEM, ARCHIVED, OPS]).map((w) => w.slug)).toEqual([
      'ops',
      'payments',
      'a-system',
    ])
  })
})

describe('resolveWorkspace', () => {
  const all = [PAYMENTS, OPS, ARCHIVED]

  it('is nothing while the list loads, or when there is none', () => {
    expect(resolveWorkspace(null, 'ops', 'ops')).toBeNull()
    expect(resolveWorkspace([], 'ops', 'ops')).toBeNull()
  })

  it('takes the address first', () => {
    expect(resolveWorkspace(all, 'payments', 'ops')?.slug).toBe('payments')
  })

  it('then the stored choice', () => {
    expect(resolveWorkspace(all, null, 'payments')?.slug).toBe('payments')
  })

  it('then the first visible workspace', () => {
    expect(resolveWorkspace(all, null, null)?.slug).toBe('ops')
  })

  it('skips an archived or vanished workspace at either step', () => {
    expect(resolveWorkspace(all, 'attic', 'payments')?.slug).toBe('payments')
    expect(resolveWorkspace(all, 'gone', 'attic')?.slug).toBe('ops')
    expect(resolveWorkspace(all, 'gone', 'also-gone')?.slug).toBe('ops')
  })
})

describe('withWorkspace', () => {
  it('sets ws and keeps the path, the other params and the fragment', () => {
    expect(withWorkspace('/runs?state=pending_approval#top', 'ops')).toBe('/runs?state=pending_approval&ws=ops#top')
    expect(withWorkspace('/runs?ws=old', 'ops')).toBe('/runs?ws=ops')
  })

  it('drops ws for no workspace', () => {
    expect(withWorkspace('/runs?ws=old&state=x', null)).toBe('/runs?state=x')
    expect(withWorkspace('/runs?ws=old', null)).toBe('/runs')
  })
})

describe('the store and the address', () => {
  beforeEach(() => {
    localStorage.clear()
    history.replaceState(null, '', '/runs')
    useWorkspaceStore.setState({ stored: null })
    useGovernance.setState({ workspaces: null, refused: {} })
    // The sync asks for the list; the test supplies it.
    vi.spyOn(useGovernance.getState(), 'load').mockImplementation(() => undefined)
  })

  function mount() {
    return renderHook(() => {
      useWorkspaceSync()
      return useCurrentWorkspace()
    })
  }

  it('resolves to the first visible workspace and writes it to the address and storage', () => {
    const view = mount()
    expect(view.result.current).toBeNull()
    act(() => {
      useGovernance.setState({ workspaces: [PAYMENTS, OPS] })
    })
    expect(view.result.current?.slug).toBe('ops')
    expect(location.search).toBe('?ws=ops')
    expect(localStorage.getItem(STORAGE_KEY)).toBe('ops')
  })

  it('follows the address on load and remembers it', () => {
    history.replaceState(null, '', '/hosts?ws=payments')
    useGovernance.setState({ workspaces: [PAYMENTS, OPS] })
    const view = mount()
    expect(view.result.current?.slug).toBe('payments')
    expect(localStorage.getItem(STORAGE_KEY)).toBe('payments')
  })

  it('a reload with no ws stays on the stored workspace', () => {
    localStorage.setItem(STORAGE_KEY, 'payments')
    useWorkspaceStore.setState({ stored: 'payments' })
    useGovernance.setState({ workspaces: [PAYMENTS, OPS] })
    const view = mount()
    expect(view.result.current?.slug).toBe('payments')
    expect(location.search).toBe('?ws=payments')
  })

  it('choosing replaces the history entry instead of pushing one', () => {
    useGovernance.setState({ workspaces: [PAYMENTS, OPS] })
    const view = mount()
    const before = history.length
    act(() => {
      useWorkspaceStore.getState().choose('payments')
    })
    expect(view.result.current?.slug).toBe('payments')
    expect(location.search).toBe('?ws=payments')
    expect(history.length).toBe(before)
    expect(localStorage.getItem(STORAGE_KEY)).toBe('payments')
  })

  it('puts ws back after a navigation drops it', () => {
    useGovernance.setState({ workspaces: [PAYMENTS, OPS] })
    mount()
    act(() => {
      history.pushState(null, '', '/hosts')
      dispatchEvent(new PopStateEvent('popstate'))
    })
    expect(location.pathname).toBe('/hosts')
    expect(location.search).toBe('?ws=ops')
  })

  it('falls back when the current workspace is archived or vanishes', () => {
    useGovernance.setState({ workspaces: [PAYMENTS, OPS] })
    history.replaceState(null, '', '/runs?ws=payments')
    const view = mount()
    expect(view.result.current?.slug).toBe('payments')
    act(() => {
      useGovernance.setState({ workspaces: [{ ...PAYMENTS, archived: true }, OPS] })
    })
    expect(view.result.current?.slug).toBe('ops')
    expect(location.search).toBe('?ws=ops')
    act(() => {
      useGovernance.setState({ workspaces: [PAYMENTS] })
    })
    expect(view.result.current?.slug).toBe('payments')
    expect(localStorage.getItem(STORAGE_KEY)).toBe('payments')
  })

  it('clears ws when there is no workspace at all', () => {
    history.replaceState(null, '', '/runs?ws=gone')
    useGovernance.setState({ workspaces: [] })
    const view = mount()
    expect(view.result.current).toBeNull()
    expect(location.search).toBe('')
  })
})
