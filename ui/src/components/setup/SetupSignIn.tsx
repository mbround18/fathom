/**
 * 043 US2: the optional sign-in step of the setup screen. Offered after a pick when an identity
 * provider is configured, or when the picked context answered 401. "Skip" keeps the kubeconfig's
 * credentials; the same controls live in Settings › Sign-in.
 */
import { ArrowRightIcon, SignInIcon } from '@phosphor-icons/react'

import { Button } from '@companion/ui'
import { SignInSettings } from '@/features/identity/SignInSettings'
import { useStore } from '@/store'

export function SetupSignIn({ context }: { context: string }) {
  const setSetup = useStore((s) => s.setSetup)
  const signedIn = useStore((s) => Object.keys(s.identity.identities).length > 0)
  const problem = useStore((s) => s.session.connectionProblems[context])
  const finish = () => setSetup({ signInStep: false, signInDone: true })

  return (
    <section className="flex max-w-2xl flex-col gap-5" data-testid="setup-sign-in-step">
      <header className="flex flex-col gap-2">
        <p className="flex items-center gap-2 font-display text-[0.65rem] tracking-[0.3em] text-accent-neon/80 uppercase">
          <SignInIcon aria-hidden="true" className="size-4 text-accent-neon" />
          Set up Fathom
        </p>
        <h2 className="font-display text-lg font-semibold tracking-wider uppercase">
          Sign in (optional)
        </h2>
        <p className="max-w-prose text-xs text-muted-foreground">
          {problem?.kind === 'Unauthorized' ? (
            <>
              <span className="font-mono">{context}</span> turned down the kubeconfig's credentials.
              Sign in with an identity provider the cluster trusts, or skip and fix the kubeconfig
              instead.
            </>
          ) : (
            <>
              Choose how Fathom signs in to <span className="font-mono">{context}</span>. Skip to
              keep using the credentials in your kubeconfig.
            </>
          )}
        </p>
      </header>
      <SignInSettings context={context} compact />
      <div className="flex flex-wrap gap-2">
        {signedIn && (
          <Button size="sm" data-testid="setup-sign-in-continue" onClick={finish}>
            Continue <ArrowRightIcon data-icon="inline-end" />
          </Button>
        )}
        <Button
          size="sm"
          variant={signedIn ? 'outline' : 'default'}
          data-testid="setup-sign-in-skip"
          onClick={finish}
        >
          Skip
        </Button>
      </div>
    </section>
  )
}
