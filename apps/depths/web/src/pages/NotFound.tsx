import { Link, Page, PageHeader } from '@companion/ui/kit'

export default function NotFound() {
  return (
    <Page name="not-found" labelledBy="not-found-title">
      <PageHeader id="not-found-title" title="Not found" description="Depths has no page at this address." />
      <Link to="/" className="self-start text-xs text-primary underline-offset-4 hover:underline">
        Back to stacks
      </Link>
    </Page>
  )
}
