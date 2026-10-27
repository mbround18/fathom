import { BrandMark } from '@companion/theme'
import { Button } from '@companion/ui'

import { rememberReturnTo } from '@/app/return-to'
import { useSession } from '@/app/session-store'
import { BRAND_GLYPH, BRAND_NAME } from '@/brand'
import { LOGIN_HREF } from '@/protocol/http'

const NOTICES = {
  expired: 'Your session ended. Sign in again to carry on where you were.',
  revoked: 'Your access changed, so this session was signed out. Sign in again.',
} as const

/**
 * Shown to anyone without a session. "Sign in" goes to the server, which sends the browser to the
 * configured identity provider; its callback lands on `/`, and the shell then returns to the page
 * asked for here (return-to.ts).
 */
export default function SignIn() {
  const notice = useSession((s) => s.notice)
  return (
    <main data-page="sign-in" className="flex min-h-full flex-col items-center justify-center gap-6 p-4">
      <BrandMark name={BRAND_NAME} glyph={BRAND_GLYPH} className="text-2xl" />
      <section
        aria-labelledby="sign-in-title"
        className="flex w-full max-w-sm flex-col gap-3 border border-border bg-card p-4"
      >
        <h1 id="sign-in-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Sign in
        </h1>
        {notice && (
          <p
            role="status"
            data-notice={notice}
            className="border border-status-warn/40 px-2 py-1 text-xs text-status-warn"
          >
            {NOTICES[notice]}
          </p>
        )}
        <p className="text-xs text-muted-foreground">Sign in to run and review your playbooks.</p>
        <Button asChild className="w-full">
          <a
            href={LOGIN_HREF}
            onClick={() => {
              rememberReturnTo(globalThis.location.pathname + globalThis.location.search)
            }}
          >
            Sign in
          </a>
        </Button>
      </section>
    </main>
  )
}
