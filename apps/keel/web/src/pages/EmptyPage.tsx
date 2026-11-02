/**
 * A page that has no content yet (spec 055 T030): the title and a line on what will live here.
 * Each story replaces its page with the real one.
 */
import { HourglassIcon } from '@phosphor-icons/react'
import { EmptyState, Page } from '@companion/ui/kit'

import { PageHeader } from '@/ui/PageHeader'

export function EmptyPage({ page, title, blurb }: { page: string; title: string; blurb: string }) {
  return (
    <Page name={page} labelledBy={`${page}-title`}>
      <PageHeader id={`${page}-title`} title={title} />
      <EmptyState icon={<HourglassIcon aria-hidden />} title="Not here yet">
        {blurb} This section arrives in a later release.
      </EmptyState>
    </Page>
  )
}
