import { lazy, Suspense, useEffect } from 'react'

import { AppRoutes, PageFallback } from './routes'
import { useSession } from './session-store'
import { Shell } from './Shell'

const SignIn = lazy(() => import('@/pages/SignIn'))
const NoAccess = lazy(() => import('@/pages/NoAccess'))

/**
 * Bootstraps from `GET /api/v1/me`, then shows exactly one of: sign-in, no-access, or the shell.
 * The gate is the session state, not the URL, so no path reaches a page the session cannot see.
 */
export function App() {
  const status = useSession((s) => s.status)
  const bootstrap = useSession((s) => s.bootstrap)

  useEffect(() => {
    void bootstrap()
  }, [bootstrap])

  switch (status) {
    case 'loading':
      return <PageFallback />
    case 'signed_out':
      return (
        <Suspense fallback={<PageFallback />}>
          <SignIn />
        </Suspense>
      )
    case 'no_access':
      return (
        <Suspense fallback={<PageFallback />}>
          <NoAccess />
        </Suspense>
      )
    case 'too_many':
      return (
        <Notice title="Too many tabs">
          Depths is open in too many tabs for this session. Close one and reload this page.
        </Notice>
      )
    case 'error':
      return (
        <Notice title="Depths is unavailable">
          The server did not answer. Reload this page to try again.
        </Notice>
      )
    case 'ready':
      return (
        <Shell>
          <AppRoutes />
        </Shell>
      )
  }
}

function Notice({ title, children }: { title: string; children: string }) {
  return (
    <main role="alert" className="flex min-h-full flex-col items-center justify-center gap-2 p-4">
      <h1 className="font-display text-sm font-semibold tracking-wider uppercase">{title}</h1>
      <p className="max-w-sm text-center text-xs text-muted-foreground">{children}</p>
    </main>
  )
}
