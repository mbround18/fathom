import { PageErrorBoundary } from '../../src/kit'
import type { Fixture } from '../fixture'

function Broken(): never {
  throw new Error('widgets is undefined')
}

const fixture: Fixture = {
  id: 'page-error-boundary',
  states: {
    thrown: () => (
      <PageErrorBoundary>
        <Broken />
      </PageErrorBoundary>
    ),
    healthy: () => (
      <PageErrorBoundary>
        <p className="text-sm">Widgets rendered without trouble.</p>
      </PageErrorBoundary>
    ),
  },
}

export default fixture
