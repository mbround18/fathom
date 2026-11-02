/**
 * `<TermTip term="trusted-signers" />`: the info icon after a glossary term in a heading, a column
 * heading or a label (spec 063 FR-021). The text comes only from `GLOSSARY` (FR-024); the shared
 * `InfoTip` does the opening and closing, and a "Learn more" link to the term's guide sits under it.
 *
 * Put it beside the heading or label, not inside it, so the heading's accessible name stays the
 * term alone. Use it at most once per section.
 */
import { InfoTip } from '@companion/ui'
import { GuideLink } from '@companion/ui/kit'

import { GLOSSARY, type TermId } from './glossary'

export function TermTip({ term, className }: { term: TermId; className?: string }) {
  const entry = GLOSSARY[term]
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
