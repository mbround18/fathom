/**
 * The route table. Every page is its own chunk (`React.lazy`), so the sign-in page never loads a
 * section and the sections load only when visited. Patterns are `/literal/:param`; a story adds
 * its detail routes (`/runs/:id`, ...) here.
 */
import { lazy, Suspense, useEffect, type ComponentType, type LazyExoticComponent } from 'react'

import { ACCOUNT_PASSKEYS, HOME } from './nav'
import { matchPath, navigate, usePath, type Params } from './router'

interface Route {
  pattern: string
  page: LazyExoticComponent<ComponentType<{ params: Params }>>
}

const ROUTES: readonly Route[] = [
  { pattern: '/runs', page: lazy(() => import('@/features/runs/RunsPage')) },
  { pattern: '/reviews', page: lazy(() => import('@/features/reviews/ReviewsPage')) },
  { pattern: '/bindings', page: lazy(() => import('@/features/bindings/BindingsPage')) },
  { pattern: '/receivers', page: lazy(() => import('@/features/receivers/ReceiversPage')) },
  { pattern: '/inventory', page: lazy(() => import('@/features/inventory/InventoryPage')) },
  { pattern: '/hosts', page: lazy(() => import('@/features/hosts/HostsPage')) },
  { pattern: '/teams', page: lazy(() => import('@/features/teams/TeamsPage')) },
  { pattern: '/users', page: lazy(() => import('@/features/users/UsersPage')) },
  { pattern: '/audit', page: lazy(() => import('@/features/audit/AuditPage')) },
  { pattern: '/migration', page: lazy(() => import('@/features/migration/MigrationPage')) },
  { pattern: ACCOUNT_PASSKEYS, page: lazy(() => import('@/features/passkeys/PasskeysPage')) },
]

const NotFound = lazy(() => import('@/pages/NotFound'))

export function PageFallback() {
  return (
    <div role="status" aria-label="Loading" data-pane-state="loading" className="p-4 text-xs text-muted-foreground">
      Loading…
    </div>
  )
}

function GoHome() {
  useEffect(() => {
    navigate(HOME, { replace: true })
  }, [])
  return <PageFallback />
}

export function AppRoutes() {
  const path = usePath()
  if (path === '/') return <GoHome />
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
