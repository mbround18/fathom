/** 033 US1: sign-in controls for one issuer: browser / device-code, progress, cancel, sign out. */
import { useStore } from '@/store'
import { Button } from '@/components/ui/button'
import type { OidcIssuerConfig } from '@/protocol/types'

import { isSignedIn } from './state'

export function SignInPanel({ issuer }: { issuer: OidcIssuerConfig }) {
  const send = useStore((s) => s.send)
  const identity = useStore((s) => s.identity.identities[issuer.id])
  const progress = useStore((s) => s.identity.progress[issuer.id])
  const error = useStore((s) => s.identity.errors[issuer.id])
  const signedIn = isSignedIn(identity)
  const id = issuer.id

  return (
    <div className="flex flex-col gap-2 text-xs" data-testid={`oidc-signin-${id}`}>
      {identity && (
        <p data-testid={`oidc-identity-${id}`} data-state={identity.state.type}>
          {identity.state.type === 'SignedOut'
            ? `Signed out${identity.state.data.reason ? `: ${identity.state.data.reason}` : ''}`
            : `Signed in as ${identity.username}`}
        </p>
      )}
      {progress?.type === 'Awaiting' && (
        <p data-testid={`oidc-awaiting-${id}`}>Waiting for the browser sign-in to finish...</p>
      )}
      {progress?.type === 'DeviceCode' && (
        <div data-testid={`oidc-device-${id}`} className="border border-border p-2">
          <p>
            Open <span className="font-mono">{progress.data.verification_uri}</span> and enter the
            code:
          </p>
          <p className="font-mono text-base tracking-widest" data-testid={`oidc-device-code-${id}`}>
            {progress.data.user_code}
          </p>
        </div>
      )}
      {progress?.type === 'Validating' && <p>Validating the ID token...</p>}
      {error && (
        <p role="alert" className="text-status-error" data-testid={`oidc-error-${id}`}>
          {error.stage}: {error.message}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        {progress ? (
          <Button
            size="xs"
            variant="outline"
            data-testid={`oidc-cancel-${id}`}
            onClick={() => send({ type: 'OidcCancelSignIn', data: { issuer_id: id } })}
          >
            Cancel
          </Button>
        ) : (
          <>
            <Button
              size="xs"
              data-testid={`oidc-signin-browser-${id}`}
              onClick={() =>
                send({ type: 'OidcSignIn', data: { issuer_id: id, method: 'Browser' } })
              }
            >
              {signedIn ? 'Sign in again' : 'Sign in with browser'}
            </Button>
            <Button
              size="xs"
              variant="outline"
              data-testid={`oidc-signin-device-${id}`}
              onClick={() =>
                send({ type: 'OidcSignIn', data: { issuer_id: id, method: 'DeviceCode' } })
              }
            >
              Use device code
            </Button>
          </>
        )}
        {signedIn && !progress && (
          <Button
            size="xs"
            variant="outline"
            data-testid={`oidc-signout-${id}`}
            onClick={() =>
              send({ type: 'OidcSignOut', data: { issuer_id: id, end_session: true } })
            }
          >
            Sign out
          </Button>
        )}
      </div>
    </div>
  )
}
