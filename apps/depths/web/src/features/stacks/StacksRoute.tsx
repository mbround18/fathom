import { lazy, Suspense } from 'react'

import type { Params } from '@/app/router'
import { PageFallback } from '@/app/routes'
import { useMedia } from '@/app/useMedia'

import { useCanSeeHost } from './host-store'
import { HostStrip } from './HostMetrics'
import { StackList } from './StackList'

const StackDetail = lazy(() => import('./StackDetail'))
const StacksOverview = lazy(() => import('./StacksOverview'))

/** From this width up the list stays beside the stack, as a sidebar. */
const SPLIT = '(min-width: 1024px)'

/**
 * `/` and `/stacks/:name` (US1). Narrow screens show the list or one stack; wide ones keep the
 * list as a sidebar and show the stack, or an overview of every stack, beside it. Both paths are
 * one route, so moving between stacks keeps the list (and its scroll and filter) mounted.
 */
export default function StacksRoute({ params }: { params: Params }) {
  const split = useMedia(SPLIT)
  const showHost = useCanSeeHost()
  const name = params.name
  const main = (
    <Suspense fallback={<PageFallback />}>
      {name === undefined ? <StacksOverview /> : <StackDetail params={params} />}
    </Suspense>
  )

  if (!split) {
    if (name !== undefined) return main
    if (!showHost) return <StackList />
    // Admins get a one-line host summary above the list; the full panel is on the wide overview.
    return (
      <div className="flex h-full min-h-0 flex-col">
        <HostStrip />
        <div className="min-h-0 flex-1">
          <StackList />
        </div>
      </div>
    )
  }
  return (
    <div data-layout="split" className="flex h-full min-h-0">
      <aside aria-label="Stack list" className="h-full w-80 shrink-0 border-r border-border xl:w-96">
        <StackList selected={name} />
      </aside>
      <div className="h-full min-h-0 min-w-0 flex-1 overflow-auto">{main}</div>
    </div>
  )
}
