/**
 * Every guide link resolves (spec 063 FR-030, SC-006).
 *
 * THE CONVENTION, for everything in `src/` that points at a guide (GuideLink, EmptyState's
 * `guide`, PageHeader's `guide`, the glossary, ...): write the `GuideRef` as an object literal with
 * string literals, `guide` first and `anchor` right after it in the same literal, e.g.
 *
 *     { guide: 'reviews', anchor: 'trusted-signers' }
 *     { guide: 'inventory' }
 *
 * The literal may span lines. This test greps `src/**\/*.{ts,tsx}` (comments included) for
 * `guide: '<slug>'` with an optional `, anchor: '<anchor>'`, and fails, naming the file and the
 * ref, when the guide or its `##`/`###` heading doesn't exist. An `anchor: '…'` anywhere else
 * (before `guide`, or on its own) fails too, since it can't be checked. A ref built at run time
 * (a variable, a template string) is invisible here: don't.
 *
 * Anchors are what the help page renders (`tests/unit/support/guides.ts`), so a ref that passes
 * here scrolls in the app. Reuse `resolveGuideRef` from there to check refs held in data.
 */
import { describe, expect, it } from 'vitest'

import { GUIDE_SLUGS } from '@/help/guides'

import { GUIDE_FILES, guideAnchors, resolveGuideRef } from './support/guides'

const SOURCES = import.meta.glob<string>('../../src/**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
})

const PAIR = /\bguide:\s*(['"])([^'"\n]+)\1(?:\s*,\s*anchor:\s*(['"])([^'"\n]+)\3)?/g
const ANCHOR = /\banchor:\s*['"]/g

interface Found {
  file: string
  guide: string
  anchor: string | undefined
}

function scan(): { refs: Found[]; strays: string[] } {
  const refs: Found[] = []
  const strays: string[] = []
  for (const [path, text] of Object.entries(SOURCES)) {
    const file = path.replace(/^\.\.\/\.\.\//, '')
    let paired = 0
    for (const m of text.matchAll(PAIR)) {
      refs.push({ file, guide: m[2] ?? '', anchor: m[4] })
      if (m[4] !== undefined) paired++
    }
    const anchors = [...text.matchAll(ANCHOR)].length
    if (anchors !== paired)
      strays.push(`${file}: ${String(anchors - paired)} anchor(s) not written right after a guide`)
  }
  return { refs, strays }
}

/** `[text](other.md#anchor)` links between guides, outside code fences. */
function guideToGuideLinks(md: string): { guide: string; anchor: string | undefined }[] {
  const prose = md.replace(/^(```|~~~)[\s\S]*?^\1/gm, '')
  return [...prose.matchAll(/\]\((?:\.\/)?([A-Za-z0-9-]+)\.md(?:#([^)\s]+))?\)/g)].map((m) => ({
    guide: m[1] ?? '',
    anchor: m[2],
  }))
}

describe('guide links (spec 063 SC-006)', () => {
  const { refs, strays } = scan()

  it('finds the refs it checks', () => {
    // guides.ts and GuideLink.tsx carry an example in their doc comments.
    expect(refs.length).toBeGreaterThan(0)
  })

  it('every GuideRef in src/ resolves', () => {
    const broken = refs.flatMap((r) => {
      const why = resolveGuideRef(r)
      const ref = r.anchor === undefined ? r.guide : `${r.guide}#${r.anchor}`
      return why === null ? [] : [`${r.file}: ${ref}: ${why}`]
    })
    expect(broken).toEqual([])
  })

  it('every anchor sits right after its guide', () => {
    expect(strays).toEqual([])
  })

  it('GuideSlug lists exactly the guides in docs/guides/keel', () => {
    const files = [...GUIDE_FILES.keys()].filter((s) => s !== 'README').sort()
    expect([...GUIDE_SLUGS].sort()).toEqual(files)
  })

  it('the README links every guide', () => {
    const linked = new Set(guideToGuideLinks(GUIDE_FILES.get('README') ?? '').map((l) => l.guide))
    expect(GUIDE_SLUGS.filter((s) => !linked.has(s))).toEqual([])
  })

  it('links between guides resolve', () => {
    const broken = [...GUIDE_FILES].flatMap(([from, md]) =>
      guideToGuideLinks(md).flatMap((l) => {
        if (l.guide === 'README') return []
        const why = resolveGuideRef(l)
        return why === null ? [] : [`${from}.md → ${l.guide}.md${l.anchor ? `#${l.anchor}` : ''}: ${why}`]
      }),
    )
    expect(broken).toEqual([])
  })

  it('every guide has a heading to link to', () => {
    for (const slug of GUIDE_SLUGS) expect(guideAnchors(slug)?.size).toBeGreaterThan(0)
  })
})
