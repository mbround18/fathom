import { describe, expect, it } from 'vitest'

import {
  isWindowsNode,
  phaseView,
  summarizeAccess,
} from '../../src/components/panels/nodeShellView'
import type { NodeShellAccessCheck, NodeShellPhase } from '../../src/protocol/types'

const check = (
  verb: string,
  outcome: NodeShellAccessCheck['outcome'],
  resource = 'pods',
  subresource: string | null = null,
): NodeShellAccessCheck => ({ verb, resource, subresource, namespace: 'default', outcome })

describe('phaseView', () => {
  it('maps every phase, mounting the terminal only once the pod runs', () => {
    const phases: NodeShellPhase[] = [
      'Creating',
      'Pending',
      'Running',
      'Attached',
      'Terminating',
      'Removed',
      'Failed',
    ]
    const views = Object.fromEntries(phases.map((p) => [p, phaseView(p, false)]))
    expect(phases.filter((p) => views[p]?.showTerminal)).toEqual(['Running', 'Attached'])
    expect(phases.filter((p) => views[p]?.busy)).toEqual(['Creating', 'Pending', 'Terminating'])
    expect(views.Failed?.tone).toBe('error')
  })

  it('offers Stop again only for a Failed session whose pod is left over', () => {
    expect(phaseView('Failed', false).canStop).toBe(false)
    const left = phaseView('Failed', true)
    expect(left.canStop).toBe(true)
    expect(left.label).toMatch(/could not be deleted/)
  })
})

describe('summarizeAccess', () => {
  it('is allowed only when every check is allowed', () => {
    expect(summarizeAccess([check('create', { type: 'Allowed' })]).allowed).toBe(true)
    expect(summarizeAccess([]).allowed).toBe(false)
  })

  it('lists denied and errored permissions, failing safe on errors', () => {
    const r = summarizeAccess([
      check('create', { type: 'Allowed' }),
      check('create', { type: 'Denied', data: { reason: 'no exec' } }, 'pods', 'exec'),
      check('get', { type: 'Error', data: { message: 'boom' } }, 'nodes'),
    ])
    expect(r.allowed).toBe(false)
    expect(r.problems).toEqual([
      'create pods/exec: no exec',
      'get nodes: could not be checked: boom',
    ])
  })
})

describe('isWindowsNode', () => {
  it('detects windows case-insensitively and treats unknown as not windows', () => {
    expect(isWindowsNode('windows')).toBe(true)
    expect(isWindowsNode('Windows')).toBe(true)
    expect(isWindowsNode('linux')).toBe(false)
    expect(isWindowsNode(undefined)).toBe(false)
  })
})
