/**
 * The Keel guides as the tests see them (spec 063 SC-006), for `guide-links.test.ts` and any other
 * test that holds a `GuideRef` (the glossary's, for one).
 *
 * Anchors come from rendering each guide with the app's own `Markdown` in `guideLinks` mode and
 * reading the `id`s of its `h2`/`h3`, so a ref resolves here exactly when it scrolls in the app.
 * The files are read through Vite's `import.meta.glob`, the same way the app bundles them.
 */
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

import Markdown from '@/help/GuideMarkdown'

const DIR = '../../../../../../docs/guides/keel/'

const RAW = import.meta.glob<string>('../../../../../../docs/guides/keel/*.md', {
  query: '?raw',
  import: 'default',
  eager: true,
})

/** Every guide's Markdown by slug (`hosts`, ...), README included. */
export const GUIDE_FILES: ReadonlyMap<string, string> = new Map(
  Object.entries(RAW).map(([path, text]) => [path.slice(DIR.length, -'.md'.length), text]),
)

/** The `id`s the help page gives a guide's `##`/`###` headings, in order. */
export function headingAnchors(md: string): string[] {
  const html = renderToStaticMarkup(createElement(Markdown, { text: md, guideLinks: true }))
  const doc = new DOMParser().parseFromString(html, 'text/html')
  return [...doc.querySelectorAll('h2[id], h3[id]')].map((h) => h.id)
}

const anchors = new Map<string, ReadonlySet<string>>()

/** A guide's anchors; `undefined` when there is no such guide. */
export function guideAnchors(guide: string): ReadonlySet<string> | undefined {
  const md = GUIDE_FILES.get(guide)
  if (md === undefined) return undefined
  let set = anchors.get(guide)
  if (set === undefined) {
    set = new Set(headingAnchors(md))
    anchors.set(guide, set)
  }
  return set
}

/** `null` when `ref` resolves; otherwise why it doesn't. */
export function resolveGuideRef(ref: { guide: string; anchor?: string | undefined }): string | null {
  if (ref.guide === 'README') return 'README is the /help index, not a guide: link the guide itself'
  const set = guideAnchors(ref.guide)
  if (set === undefined) return `no guide docs/guides/keel/${ref.guide}.md`
  if (ref.anchor !== undefined && !set.has(ref.anchor))
    return `no heading #${ref.anchor} in ${ref.guide}.md (it has: ${[...set].join(', ') || 'none'})`
  return null
}
