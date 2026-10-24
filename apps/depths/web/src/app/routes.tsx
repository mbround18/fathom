/**
 * The route table. Every page is its own chunk (`React.lazy`), so the sign-in page never loads
 * the stack views and vice versa. Add a story's routes here; patterns are `/literal/:param`.
 */
import { lazy, Suspense, type ComponentType, type LazyExoticComponent } from 'react'

import { matchPath, usePath, type Params } from './router'

interface Route {
  pattern: string
  page: LazyExoticComponent<ComponentType<{ params: Params }>>
}

const ROUTES: readonly Route[] = [
  { pattern: '/', page: lazy(() => import('@/features/stacks/StacksRoute')) },
  { pattern: '/stacks/:name', page: lazy(() => import('@/features/stacks/StackDetail')) },
]

const NotFound = lazy(() => import('@/pages/NotFound'))

export function PageFallback() {
  return (
    <div role="status" aria-label="Loading" data-pane-state="loading" className="p-4 text-xs text-muted-foreground">
      Loading…
    </div>
  )
}

export function AppRoutes() {
  const path = usePath()
  for (const { pattern, page: Page } of ROUTES) {
    const params = matchPath(pattern, path)
    if (params)
      return (
        <Suspense fallback={<PageFallback />}>
          <Page key={pattern} params={params} />
        </Suspense>
      )
  }
  return (
    <Suspense fallback={<PageFallback />}>
      <NotFound />
    </Suspense>
  )
}
