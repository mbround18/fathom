import { BrandMark } from '@companion/theme'
import { Button } from '@companion/ui'

import { useSession } from '@/app/session-store'
import { BRAND_GLYPH, BRAND_NAME } from '@/brand'

/**
 * FR-031: a `pending` person, a `user` with no stacks, or a socket closed 4403. Says nothing
 * about the host: no stacks, counts, names or settings. The only action is signing out.
 */
export default function NoAccess() {
  const signOut = useSession((s) => s.signOut)
  return (
    <main
      data-page="no-access"
      className="flex min-h-full flex-col items-center justify-center gap-6 p-4"
    >
      <BrandMark name={BRAND_NAME} glyph={BRAND_GLYPH} className="text-2xl" />
      <section
        aria-labelledby="no-access-title"
        className="flex w-full max-w-sm flex-col gap-3 border border-border bg-card p-4"
      >
        <h1
          id="no-access-title"
          className="font-display text-sm font-semibold tracking-wider uppercase"
        >
          You don&apos;t currently have access
        </h1>
        <p className="text-xs text-muted-foreground">
          You are signed in, but no stacks are shared with you yet. Ask an administrator to give
          you access, then reload this page.
        </p>
        <Button variant="outline" onClick={() => void signOut()}>
          Sign out
        </Button>
      </section>
    </main>
  )
}
