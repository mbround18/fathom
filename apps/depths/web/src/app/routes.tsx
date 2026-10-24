/**
 * The route table. Every page is its own chunk (`React.lazy`), so the sign-in page never loads
 * the stack views and vice versa. Add a story's routes here; patterns are `/literal/:param`.
 *
 * `admin` routes do not exist for anyone else: a non-admin gets the not-found page, so the URL
 * says nothing about what is there. The server refuses their commands regardless (FR-032).
 */
import { lazy, Suspense, type ComponentType, type LazyExoticComponent } from 'react'

import { matchPath, usePath, type Params } from './router'
import { useSession } from './session-store'

interface Route {
  pattern: string
  page: LazyExoticComponent<ComponentType<{ params: Params }>>
  admin?: boolean
  /** Routes in one group share a page instance, so moving between them does not remount it. */
  group?: string
}

const stacksPage = lazy(() => import('@/features/stacks/StacksRoute'))

const ROUTES: readonly Route[] = [
  { pattern: '/', page: stacksPage, group: 'stacks' },
  { pattern: '/stacks/:name', page: stacksPage, group: 'stacks' },
  { pattern: '/stacks/:name/edit', page: lazy(() => import('@/features/editor/EditStack')) },
  { pattern: '/new', page: lazy(() => import('@/features/stacks/CreateStack')) },
  { pattern: '/admin/users', page: lazy(() => import('@/features/users/UsersView')), admin: true },
  { pattern: '/admin/audit', page: lazy(() => import('@/features/audit/AuditLog')), admin: true },
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
  const isAdmin = useSession((s) => s.me?.role === 'admin')
  for (const { pattern, page: Page, admin, group } of ROUTES) {
    if (admin && !isAdmin) continue
    const params = matchPath(pattern, path)
    if (params)
      return (
        <Suspense fallback={<PageFallback />}>
          <Page key={group ?? pattern} params={params} />
        </Suspense>
      )
  }
  return (
    <Suspense fallback={<PageFallback />}>
      <NotFound />
    </Suspense>
  )
}
