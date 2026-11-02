import { Button } from '../../src/button'
import { PageHeader, Select } from '../../src/kit'
import type { Fixture } from '../fixture'

const actions = (
  <>
    <Button type="button" variant="outline" size="sm">
      Export
    </Button>
    <Button type="button" size="sm">
      Add widget
    </Button>
  </>
)

const fixture: Fixture = {
  id: 'page-header',
  states: {
    page: () => <PageHeader id="widgets-title" title="Widgets" actions={actions} />,
    intro: () => (
      <PageHeader id="widgets-title" title="Widgets" description="Everything the harness lists, newest first." actions={actions} />
    ),
    guide: () => (
      <PageHeader
        id="widgets-title"
        title="Widgets"
        description="Widgets do the work the harness keeps track of. Each one has a type, a name and a place it runs, and the guide covers all three in more detail than fits here."
        guide={{ guide: 'widgets' }}
        actions={actions}
      >
        <label htmlFor="widgets-show" className="text-xs text-muted-foreground">
          Show
        </label>
        <Select id="widgets-show" defaultValue="all">
          <option value="all">All widgets</option>
          <option value="running">Running</option>
        </Select>
      </PageHeader>
    ),
  },
}

export default fixture
