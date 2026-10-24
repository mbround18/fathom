// T068: the tab layout survives a round trip through storage, drops what it does not understand,
// and carries on when storage throws.
import { describe, expect, it } from 'vitest'

import {
  addTab,
  closeTab,
  EMPTY_LAYOUT,
  focusTab,
  layoutKey,
  loadLayout,
  MAX_PANES,
  MAX_TABS,
  moveTab,
  orderedTabs,
  reorderTab,
  restore,
  saveLayout,
  serialise,
  type Layout,
  type StorageLike,
  type TabSpec,
} from '@/features/tabs/layout'
import { lineTime, stripAnsi } from '@/features/tabs/logtext'

const logs = (id: string, container: string): TabSpec => ({ id, kind: 'logs', target: { kind: 'container', container } })
const shell = (id: string, container: string): TabSpec => ({ id, kind: 'shell', container })
const stats = (id: string, stack: string): TabSpec => ({ id, kind: 'stats', target: { kind: 'stack', stack } })

function three(): Layout {
  let l = EMPTY_LAYOUT
  l = addTab(l, logs('a', 'web-1'))
  l = addTab(l, shell('b', 'web-1'))
  l = addTab(l, stats('c', 'shop'))
  return l
}

class MemoryStorage implements StorageLike {
  map = new Map<string, string>()
  getItem(k: string): string | null {
    return this.map.get(k) ?? null
  }
  setItem(k: string, v: string): void {
    this.map.set(k, v)
  }
}

describe('layout edits', () => {
  it('adds tabs to the focused pane and opens the dock', () => {
    const l = three()
    expect(l.panes).toHaveLength(1)
    expect(l.panes[0]?.tabs).toEqual(['a', 'b', 'c'])
    expect(l.focused).toBe('c')
    expect(l.open).toBe(true)
  })

  it('focuses an existing log or stats tab rather than opening a second, but never dedupes shells', () => {
    let l = three()
    l = addTab(l, logs('x', 'web-1'))
    expect(l.tabs).toHaveLength(3)
    expect(l.focused).toBe('a')
    l = addTab(l, shell('y', 'web-1'))
    expect(l.tabs).toHaveLength(4)
  })

  it('caps the number of tabs', () => {
    let l = EMPTY_LAYOUT
    for (let i = 0; i < MAX_TABS + 4; i++) l = addTab(l, shell(`s${String(i)}`, 'web-1'))
    expect(l.tabs).toHaveLength(MAX_TABS)
  })

  it('splits into new panes up to the cap, and an emptied pane goes', () => {
    let l = three()
    l = moveTab(l, 'c', 'new')
    l = moveTab(l, 'b', 'new')
    expect(l.panes.map((p) => p.tabs)).toEqual([['a'], ['c'], ['b']])
    expect(l.panes).toHaveLength(MAX_PANES)
    // A pane of one tab cannot be split, and there is no room anyway.
    expect(moveTab(l, 'a', 'new')).toBe(l)
    l = moveTab(l, 'c', 0)
    expect(l.panes.map((p) => p.tabs)).toEqual([['a', 'c'], ['b']])
    expect(l.focused).toBe('c')
  })

  it('reorders within a pane and keeps the order in the narrow strip', () => {
    let l = reorderTab(three(), 'c', -1)
    expect(l.panes[0]?.tabs).toEqual(['a', 'c', 'b'])
    expect(reorderTab(l, 'a', -1)).toBe(l)
    l = moveTab(l, 'a', 'new')
    expect(orderedTabs(l).map((t) => t.id)).toEqual(['c', 'b', 'a'])
  })

  it('closing the active tab activates its neighbour; closing the last closes the dock', () => {
    let l = focusTab(three(), 'b')
    l = closeTab(l, 'b')
    expect(l.panes[0]?.active).toBe('c')
    l = closeTab(closeTab(l, 'a'), 'c')
    expect(l.tabs).toEqual([])
    expect(l.panes).toEqual([])
    expect(l.open).toBe(false)
    expect(l.focused).toBeNull()
  })
})

describe('serialise and restore', () => {
  it('round-trips a layout', () => {
    let l = moveTab(three(), 'c', 'new')
    l = { ...l, sizes: { 'pane-1': 60, 'pane-2': 40, dock: 35 } }
    expect(restore(serialise(l))).toEqual(l)
  })

  it('keeps a shell tab that is waiting to offer a new session', () => {
    const l = { ...three(), tabs: three().tabs.map((t) => (t.kind === 'shell' ? { ...t, offerNew: true } : t)) }
    const back = restore(serialise(l)).tabs.find((t) => t.kind === 'shell')
    expect(back).toMatchObject({ offerNew: true })
  })

  it('drops unknown, malformed and duplicate tabs, and repairs the panes', () => {
    const text = JSON.stringify({
      v: 1,
      tabs: [
        { id: 'a', kind: 'logs', target: { kind: 'container', container: 'web-1' } },
        { id: 'b', kind: 'browser', url: 'https://example.com' },
        { id: 'c', kind: 'shell', container: '../../etc', shell: 'zsh' },
        { id: 'd', kind: 'stats', target: { kind: 'stack', stack: 'shop' } },
        { id: 'a', kind: 'shell', container: 'dup' },
        { id: 'e', kind: 'shell', container: 'db-1', shell: 'rm -rf' },
        'nonsense',
      ],
      panes: [
        { id: 'pane-1', tabs: ['b', 'c', 'a', 'a'], active: 'b' },
        { id: 'pane-2', tabs: ['zzz'], active: 'zzz' },
      ],
      sizes: { 'pane-1': 50, bad: -3, worse: 'x', huge: 500 },
      focused: 'b',
      open: true,
    })
    const l = restore(text)
    expect(l.tabs.map((t) => t.id)).toEqual(['a', 'd', 'e'])
    expect(l.tabs.find((t) => t.id === 'e')).toEqual({ id: 'e', kind: 'shell', container: 'db-1' })
    // pane-2 held nothing real and went; the loose tabs joined the first pane.
    expect(l.panes).toEqual([{ id: 'pane-1', tabs: ['a', 'd', 'e'], active: 'a' }])
    expect(l.sizes).toEqual({ 'pane-1': 50 })
    expect(l.focused).toBe('a')
    expect(l.open).toBe(true)
  })

  it('treats garbage, other versions and nothing as an empty layout', () => {
    expect(restore(null)).toEqual(EMPTY_LAYOUT)
    expect(restore('{not json')).toEqual(EMPTY_LAYOUT)
    expect(restore('[]')).toEqual(EMPTY_LAYOUT)
    expect(restore(JSON.stringify({ ...three(), v: 2 }))).toEqual(EMPTY_LAYOUT)
  })
})

describe('storage', () => {
  it('saves and loads per person', () => {
    const s = new MemoryStorage()
    expect(saveLayout('github:1', three(), s)).toBe(true)
    expect(s.map.has(layoutKey('github:1'))).toBe(true)
    expect(loadLayout('github:1', s).tabs).toHaveLength(3)
    expect(loadLayout('github:2', s)).toEqual(EMPTY_LAYOUT)
  })

  it('carries on when storage throws or is missing', () => {
    const angry: StorageLike = {
      getItem: () => {
        throw new DOMException('denied', 'SecurityError')
      },
      setItem: () => {
        throw new DOMException('full', 'QuotaExceededError')
      },
    }
    expect(saveLayout('github:1', three(), angry)).toBe(false)
    expect(loadLayout('github:1', angry)).toEqual(EMPTY_LAYOUT)
    expect(saveLayout('github:1', three(), null)).toBe(false)
    expect(loadLayout('github:1', null)).toEqual(EMPTY_LAYOUT)
  })
})

describe('log text', () => {
  it('strips colour, cursor and title escapes and stray control bytes', () => {
    expect(stripAnsi('\u001b[31mred\u001b[0m \u001b]0;title\u0007ok\u0000\u0008\tend')).toBe('red ok\tend')
    expect(stripAnsi('a‮b')).toBe('ab')
  })

  it('reads a leading timestamp, with or without a service prefix', () => {
    expect(lineTime('2026-10-23T10:00:00.123Z hello')?.ms).toBe(Date.parse('2026-10-23T10:00:00.123Z'))
    expect(lineTime('web | 2026-10-23T10:00:01Z hi')?.ms).toBe(Date.parse('2026-10-23T10:00:01Z'))
    expect(lineTime('no time here')).toBeNull()
  })
})
