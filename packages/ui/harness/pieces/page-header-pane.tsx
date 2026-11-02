import { useState } from 'react'

import { Button } from '../../src/button'
import { PageHeader } from '../../src/kit'
import type { Fixture } from '../fixture'

function Pane({ long = false }: { long?: boolean }) {
  const [drops, setDrops] = useState(0)
  return (
    <section aria-labelledby="pane-title" className="flex h-64 min-w-0 flex-col border border-border">
      <PageHeader
        size="pane"
        id="pane-title"
        title={long ? 'Deployments in the production-eu-west-1 namespace with a long name' : 'Pods'}
        lead={
          <span aria-hidden="true" className="text-xs text-accent-neon">
            01
          </span>
        }
        count={<span aria-label="12 items">12</span>}
        actions={
          <Button variant="ghost" size="xs">
            Refresh
          </Button>
        }
        rootProps={{
          draggable: true,
          'data-drag-handle': 'pods',
          onDragEnd: () => {
            setDrops((d) => d + 1)
          },
        }}
      />
      <p className="p-2 text-xs text-muted-foreground">
        Pane body. Drags: <span data-drags>{drops}</span>
      </p>
    </section>
  )
}

const fixture: Fixture = {
  id: 'page-header-pane',
  states: {
    default: () => <Pane />,
    'long-title': () => <Pane long />,
    'title-only': () => (
      <section aria-labelledby="bare-title" className="border border-border">
        <PageHeader size="pane" id="bare-title" title="Events" />
      </section>
    ),
  },
}

export default fixture
