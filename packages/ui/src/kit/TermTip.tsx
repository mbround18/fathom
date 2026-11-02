/// <reference types="vite/client" />
/**
 * `<TermTip term="trusted-signers" />`: the info icon after a glossary term in a heading, a column
 * heading or a label (spec 063 FR-021). The text comes only from the app's glossary (FR-024,
 * `KitProvider`); the shared `InfoTip` does the opening and closing, and a "Learn more" link to the
 * term's guide sits under it. A term the glossary lacks renders nothing (and warns in development).
 *
 * Put it beside the heading or label, not inside it, so the heading's accessible name stays the
 * term alone. Use it at most once per section.
 */
import { InfoTip } from '../info-tip'

import { GuideLink } from './GuideLink'
import { useKit } from './provider'

export function TermTip({ term, className }: { term: string; className?: string }) {
  const entry = useKit().glossary?.[term]
  if (entry === undefined) {
    if (import.meta.env.DEV) console.warn(`TermTip: no glossary entry for "${term}"`)
    return null
  }
  return (
    <InfoTip
      label={entry.term.toLowerCase()}
      data-infotip={term}
      contentData={{ 'data-infotip-content': term }}
      footer={entry.guide && <GuideLink to={entry.guide} />}
      {...(className === undefined ? {} : { className })}
    >
      {entry.text}
    </InfoTip>
  )
}
