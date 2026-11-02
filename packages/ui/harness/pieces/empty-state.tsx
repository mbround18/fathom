import { CubeIcon } from '@phosphor-icons/react'

import { Button } from '../../src/button'
import { EmptyState, Page, PageHeader } from '../../src/kit'
import type { Fixture } from '../fixture'

const STEPS = ['Pick a widget type.', 'Name it and choose where it runs.', 'Save it, and it shows up here.']

const guided = () => (
  <EmptyState
    page="widgets"
    title="No widgets yet"
    reason="Widgets do the work this list keeps track of."
    steps={STEPS}
    action={<Button type="button">Add widget</Button>}
    guide={{ guide: 'widgets', anchor: 'add-a-widget' }}
  />
)

const fixture: Fixture = {
  id: 'empty-state',
  states: {
    guided,
    plain: () => <EmptyState icon={<CubeIcon aria-hidden />} title="No widgets yet" reason="Widgets you add show up here." />,
    restricted: () => (
      <EmptyState
        page="widgets"
        title="No widgets yet"
        reason="Widgets do the work this list keeps track of."
        whoCan="An owner or admin"
        verb="add widgets"
        guide={{ guide: 'widgets' }}
      />
    ),
    page: () => (
      <Page name="widgets" labelledBy="widgets-title">
        <PageHeader id="widgets-title" title="Widgets" description="Everything the harness lists." />
        {guided()}
      </Page>
    ),
  },
}

export default fixture
