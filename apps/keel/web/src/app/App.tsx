/**
 * Keel's web shell (spec 055 T030). Bootstraps the session (session-store.ts), then shows exactly
 * one of: sign-in, a notice, or the shell. The gate is the session state, not the URL, so no path
 * reaches a page without a session.
 */
import { Button } from '@companion/ui'
import { lazy, Suspense, useEffect, type ReactNode } from 'react'

import { AppRoutes, PageFallback } from './routes'
import { useSession } from './session-store'
import { Shell } from './Shell'

const SignIn = lazy(() => import('@/pages/SignIn'))

export function App() {
  const status = useSession((s) => s.status)
  const bootstrap = useSession((s) => s.bootstrap)

  useEffect(() => {
    bootstrap()
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
    case 'outdated':
      return (
        <Notice title="Keel was updated" kind="outdated">
          This page is older than the server. Reload to get the new version.
        </Notice>
      )
    case 'error':
      return (
        <Notice title="Keel is unavailable" kind="error">
          The server could not load your account. Reload this page to try again.
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

function Notice({ title, kind, children }: { title: string; kind: string; children: ReactNode }) {
  return (
    <main
      role="alert"
      data-notice={kind}
      className="flex min-h-full flex-col items-center justify-center gap-3 p-4"
    >
      <h1 className="font-display text-sm font-semibold tracking-wider uppercase">{title}</h1>
      <p className="max-w-sm text-center text-xs text-muted-foreground">{children}</p>
      <Button size="sm" onClick={() => globalThis.location.reload()}>
        Reload
      </Button>
    </main>
  )
}
