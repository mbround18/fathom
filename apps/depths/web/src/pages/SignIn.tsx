import { BrandMark } from '@companion/theme'
import { Button } from '@companion/ui'

import { useSession } from '@/app/session-store'
import { BRAND_GLYPH, BRAND_NAME } from '@/brand'
import { loginHref } from '@/protocol/http'
import type { Provider } from '@/protocol/generated'

const LABELS: Record<Provider, string> = {
  oidc: 'Sign in with your identity provider',
  github: 'Sign in with GitHub',
  discord: 'Sign in with Discord',
}

/** Shown to anyone without a session. Lists the providers the server says are configured. */
export default function SignIn() {
  const providers = useSession((s) => s.providers)
  return (
    <main
      data-page="sign-in"
      className="flex min-h-full flex-col items-center justify-center gap-6 p-4"
    >
      <BrandMark name={BRAND_NAME} glyph={BRAND_GLYPH} className="text-2xl" />
      <section
        aria-labelledby="sign-in-title"
        className="flex w-full max-w-sm flex-col gap-3 border border-border bg-card p-4"
      >
        <h1 id="sign-in-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Sign in
        </h1>
        <p className="text-xs text-muted-foreground">Sign in to manage your stacks.</p>
        <ul className="flex flex-col gap-2">
          {providers.length === 0 ? (
            <li>
              <Button asChild className="w-full">
                <a href={loginHref()}>Sign in</a>
              </Button>
            </li>
          ) : (
            providers.map((p) => (
              <li key={p}>
                <Button asChild className="w-full" data-provider={p}>
                  <a href={loginHref(p)}>{LABELS[p]}</a>
                </Button>
              </li>
            ))
          )}
        </ul>
      </section>
    </main>
  )
}
