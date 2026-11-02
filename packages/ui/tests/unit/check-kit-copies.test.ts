/**
 * `scripts/check-kit-copies.mjs` (spec 065 FR-015, research R10), run against the fixture tree in
 * `fixtures/check-kit-copies`: a small kit index, an app that only uses the kit, an app that keeps
 * its own copies, and allow-lists. App fixtures are `.jsx`, which neither the typecheck nor ESLint
 * reads, so they can hold the shapes the check must tell apart.
 */
import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const SCRIPT = resolve(import.meta.dirname, '../../../../scripts/check-kit-copies.mjs')
const ROOT = resolve(import.meta.dirname, 'fixtures/check-kit-copies')

function check(...args: string[]): { status: number | null; out: string; err: string } {
  const run = spawnSync(process.execPath, [SCRIPT, '--root', ROOT, ...args], { encoding: 'utf8' })
  return { status: run.status, out: run.stdout, err: run.stderr }
}

describe('check-kit-copies', () => {
  it('passes an app that only imports, wraps, lazily loads or nests kit pieces', () => {
    const { status, out, err } = check('--scan', 'apps/clean', '--allow', 'allow/none.json')
    expect(err).toBe('')
    expect(status).toBe(0)
    expect(out).toContain('no app copies of the 6 kit exports')
  })

  it('names every top-level copy, its line and @companion/ui/kit', () => {
    const { status, err } = check('--scan', 'apps/copies', '--allow', 'allow/none.json')
    expect(status).toBe(1)
    const lines = err.split('\n').filter((l) => l.includes(' defines '))
    expect(lines.sort()).toEqual([
      'apps/copies/copies.ts:1 defines EmptyState; import it from @companion/ui/kit',
      'apps/copies/copies.ts:4 defines H2; import it from @companion/ui/kit',
      'apps/copies/more.jsx:1 defines ConfirmDialog; import it from @companion/ui/kit',
      'apps/copies/more.jsx:11 defines LoadFailed; import it from @companion/ui/kit',
      'apps/copies/more.jsx:3 defines Markdown; import it from @companion/ui/kit',
    ])
    expect(err).not.toContain('more.test.jsx')
  })

  it('scans several roots at once', () => {
    const { status, err } = check('--scan', 'apps/clean', '--scan', 'apps/copies', '--allow', 'allow/none.json')
    expect(status).toBe(1)
    expect(err).toContain('5 copies of kit pieces')
  })

  it('accepts copies that are on the allow-list with a reason', () => {
    const { status, err } = check('--scan', 'apps/copies', '--allow', 'allow/ok.json')
    expect(err).toBe('')
    expect(status).toBe(0)
  })

  it('fails on a stale allow-list entry', () => {
    const { status, err } = check('--scan', 'apps/clean', '--allow', 'allow/stale.json')
    expect(status).toBe(1)
    expect(err).toContain('stale entry (apps/clean/screen.jsx EmptyState) matches nothing')
  })

  it('fails on an allow-list entry without a reason', () => {
    const { status, err } = check('--scan', 'apps/copies', '--allow', 'allow/no-reason.json')
    expect(status).toBe(1)
    expect(err).toContain('(apps/copies/copies.ts EmptyState): needs a non-empty "reason"')
    // The entry does not count, so the copy is still reported.
    expect(err).toContain('apps/copies/copies.ts:1 defines EmptyState')
  })

  it('refuses to run without a kit index', () => {
    const run = spawnSync(process.execPath, [SCRIPT, '--root', resolve(ROOT, 'apps')], { encoding: 'utf8' })
    expect(run.status).toBe(2)
    expect(run.stderr).toContain('cannot read the kit index')
  })
})
