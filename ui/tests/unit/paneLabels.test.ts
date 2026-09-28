/**
 * 044 (T052a): a waiting cascade column's header names its job. The rest of `paneTitle` is
 * unchanged, and the cases here pin the distinction the operator reads across the row.
 */
import { describe, expect, it } from 'vitest'

import { cascadeWaitingTitle, paneTitle } from '@/components/shell/labels'
import type { PaneState } from '@/store'

const inspector = (target?: PaneState['target']): PaneState =>
  ({ id: 'inspector-1', kind: 'inspector', ...(target ? { target } : {}) }) as PaneState

describe('cascadeWaitingTitle', () => {
  it('names what fills the first cascade column: the row the operator picks', () => {
    expect(cascadeWaitingTitle('1')).toBe('Selected item')
  })

  it('names the follow that fills every later column', () => {
    expect(cascadeWaitingTitle('2')).toBe('Next level')
    expect(cascadeWaitingTitle('4')).toBe('Next level')
  })

  it('never returns the generic widget name a waiting column used to show', () => {
    for (const step of ['1', '2', '3', '4', '5']) {
      expect(cascadeWaitingTitle(step)).not.toBe('Inspector')
    }
  })
})

describe('paneTitle for inspectors', () => {
  it('still names the open object', () => {
    expect(
      paneTitle(inspector({ section: 'pods', kind: 'Pod', namespace: 'default', name: 'api-0' })),
    ).toBe('api-0')
  })

  it('still falls back to "Inspector" below wide, where there is no cascade', () => {
    expect(paneTitle(inspector())).toBe('Inspector')
  })
})
