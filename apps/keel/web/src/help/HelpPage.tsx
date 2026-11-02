/**
 * Help (spec 063 R6): `/help` shows the guides' index (`docs/guides/keel/README.md`) and
 * `/help/:guide` one guide, both through the safe Markdown renderer in `guideLinks` mode. The
 * guide's `#` heading becomes the page title; a `#anchor` in the URL scrolls to its heading. An
 * unknown guide is the ordinary Not found page.
 */
import { use, useEffect } from 'react'
import { useHash, type Params } from '@companion/ui/kit'

import NotFound from '@/pages/NotFound'
import Markdown from '@/ui/Markdown'
import { Page } from '@/ui/Page'
import { PageHeader } from '@/ui/PageHeader'

import { isGuideSlug, loadGuide, type GuideSlug } from './guides'

/** One promise per guide, kept, so `use()` sees the same one on every render. */
const loaded = new Map<GuideSlug | 'README', Promise<string>>()

function guideText(slug: GuideSlug | 'README'): Promise<string> {
  let p = loaded.get(slug)
  if (p === undefined) {
    p = loadGuide(slug)
    loaded.set(slug, p)
  }
  return p
}

/** The guide's opening `# Title` line, and the Markdown after it. */
function splitTitle(md: string): { title: string | null; body: string } {
  const m = /^\s*#[ \t]+(.+?)[ \t#]*(?:\r?\n|$)/.exec(md)
  if (!m) return { title: null, body: md }
  return { title: m[1] ?? null, body: md.slice(m[0].length) }
}

function Guide({ slug }: { slug: GuideSlug | 'README' }) {
  const { title, body } = splitTitle(use(guideText(slug)))
  const hash = useHash()

  useEffect(() => {
    if (hash === '') return
    document.getElementById(hash)?.scrollIntoView({ block: 'start' })
  }, [hash, body])

  return (
    <Page name="help" labelledBy="help-title">
      <PageHeader id="help-title" title={title ?? 'Help'} />
      <article data-help-guide={slug} className="max-w-3xl">
        <Markdown text={body} guideLinks />
      </article>
    </Page>
  )
}

export default function HelpPage({ params }: { params: Params }) {
  const guide = params.guide
  if (guide === undefined) return <Guide slug="README" />
  return isGuideSlug(guide) ? <Guide key={guide} slug={guide} /> : <NotFound />
}
