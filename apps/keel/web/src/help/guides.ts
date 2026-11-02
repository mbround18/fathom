/**
 * The Keel guides, bundled into the app (spec 063 R6). They are the Markdown files in
 * `docs/guides/keel/`, the same ones the repository shows, so the help a self-hosted Keel serves
 * matches the version it runs and works offline. Each guide is its own lazy chunk: nothing here
 * loads a guide until `/help` asks for it.
 *
 * Point at a guide with a `GuideRef` object literal, `guide` first and `anchor` after it, e.g.
 * `{ guide: 'hosts', anchor: 'enroll-a-host' }`. `tests/unit/guide-links.test.ts` finds every such
 * literal in `src/` and fails when its guide or heading doesn't exist (SC-006).
 */
import type { GuideRef as KitGuideRef } from '@companion/ui/kit'

/** One per `docs/guides/keel/*.md` file except README, which is the `/help` index. */
export const GUIDE_SLUGS = [
  'getting-started',
  'hosts',
  'receivers',
  'bindings',
  'runs',
  'reviews',
  'inventory',
  'enrollment-template',
  'oidc-roles',
  'system-workspace',
  'updates',
] as const

export type GuideSlug = (typeof GUIDE_SLUGS)[number]

/** A guide, and optionally one of its `##`/`###` headings, as `slug(heading)` (R7). */
export interface GuideRef {
  guide: GuideSlug
  anchor?: string
}

export function isGuideSlug(s: string): s is GuideSlug {
  return (GUIDE_SLUGS as readonly string[]).includes(s)
}

/**
 * The in-app route for a guide: `/help/<slug>`, or `/help` for the README index, plus `#<anchor>`
 * when given. It takes the kit's `GuideRef` (spec 065), whose `guide` is any string, so it can be
 * the kit's `guideHref`.
 */
export function guideHref(ref: KitGuideRef): string {
  const hash = ref.anchor === undefined ? '' : `#${ref.anchor}`
  return ref.guide === 'README' ? `/help${hash}` : `/help/${ref.guide}${hash}`
}

/** Whether a Markdown link target names a guide, the README index included. */
export function isGuide(s: string): boolean {
  return s === 'README' || isGuideSlug(s)
}

const DIR = '../../../../../docs/guides/keel/'

const LOADERS = import.meta.glob<string>('../../../../../docs/guides/keel/*.md', {
  query: '?raw',
  import: 'default',
})

/** A guide's Markdown source; `README` is the index. */
export function loadGuide(slug: GuideSlug | 'README'): Promise<string> {
  const load = LOADERS[`${DIR}${slug}.md`]
  return load ? load() : Promise.reject(new Error(`no guide named ${slug}`))
}
