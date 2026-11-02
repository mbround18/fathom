/**
 * "Learn more": a link to a guide in the app's help (spec 063 R6, contracts/ui.md). It stays in
 * the tab, so the page someone came from is one Back away. The route comes from the app's
 * `guideHref` (`KitProvider`); an app without guides gets nothing here. Pass the ref as a literal,
 * `<GuideLink to={{ guide: 'hosts', anchor: 'enroll-a-host' }} />`, so the app's tests can check
 * that it resolves.
 */
import { ArrowRightIcon } from '@phosphor-icons/react'
import type { ReactNode } from 'react'

import { Link as KitLink } from './Link'
import { useKit, type GuideRef } from './provider'

export function GuideLink({ to, children = 'Learn more' }: { to: GuideRef; children?: ReactNode }) {
  const { guideHref, Link = KitLink } = useKit()
  if (guideHref === undefined) return null
  return (
    <Link
      to={guideHref(to)}
      data-guide-link={to.guide}
      className="inline-flex items-center gap-1 text-xs text-primary underline-offset-4 hover:underline focus-visible:underline focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none"
    >
      {children}
      <ArrowRightIcon aria-hidden className="size-3" />
    </Link>
  )
}
