import { afterEach, describe, expect, it, vi } from 'vitest'

import { navigate, setNavigationGuard } from '@/app/router'
import { problemText, serverProblems, yamlProblems } from '@/features/editor/problems'

describe('yamlProblems', () => {
  it('finds nothing in valid YAML', () => {
    expect(yamlProblems('services:\n  app:\n    image: nginx:1.29\n')).toEqual([])
  })

  it('anchors a broken indent to its line', () => {
    const problems = yamlProblems('services:\n  app:\n    image: nginx\n   ports: []\n')
    expect(problems.length).toBeGreaterThan(0)
    expect(problems[0]?.line).toBe(4)
    expect(problems[0]?.source).toBe('yaml')
  })

  it('reports one problem per line, the first message only', () => {
    const problems = yamlProblems('a: [\n')
    const lines = problems.map((p) => p.line)
    expect(new Set(lines).size).toBe(lines.length)
    for (const p of problems) expect(p.message).not.toContain('\n')
  })
})

describe('serverProblems', () => {
  it('keeps the server line and message', () => {
    const p = serverProblems([
      { file: 'compose', line: 3, message: 'service "app" has no image' },
      { file: 'compose', line: null, message: 'project has no services' },
    ])
    expect(p.map(problemText)).toEqual(['Line 3: service "app" has no image', 'project has no services'])
    expect(p.every((x) => x.source === 'server')).toBe(true)
  })
})

describe('navigation guard', () => {
  afterEach(() => {
    setNavigationGuard(null)
    navigate('/')
  })

  it('stays put when the guard refuses, and moves when it allows', () => {
    navigate('/')
    const guard = vi.fn(() => false)
    setNavigationGuard(guard)
    navigate('/stacks/web')
    expect(guard).toHaveBeenCalledOnce()
    expect(globalThis.location.pathname).toBe('/')

    guard.mockReturnValue(true)
    navigate('/stacks/web')
    expect(globalThis.location.pathname).toBe('/stacks/web')
  })

  it('does not ask for a navigation to the current page', () => {
    navigate('/stacks/web')
    const guard = vi.fn(() => false)
    setNavigationGuard(guard)
    navigate('/stacks/web')
    expect(guard).not.toHaveBeenCalled()
  })
})
