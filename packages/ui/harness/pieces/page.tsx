import { H2, Page, PageHeader } from '../../src/kit'
import type { Fixture } from '../fixture'

const fixture: Fixture = {
  id: 'page',
  states: {
    default: () => (
      <Page name="widgets" labelledBy="widgets-title">
        <PageHeader id="widgets-title" title="Widgets" description="Everything the harness lists." />
        <section aria-labelledby="recent-title" className="flex flex-col gap-2">
          <h2 id="recent-title" className={H2}>
            Recent
          </h2>
          <p className="text-sm text-muted-foreground">Blue widget was added a minute ago.</p>
        </section>
      </Page>
    ),
  },
}

export default fixture
