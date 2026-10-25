import { Button } from '@companion/ui'
import { HostedPanel } from './HostedPanel'

/** Shown while unauthenticated (or before the redirect to the identity provider lands). */
export function SignInGate({
  loginUrl = '/auth/login',
  returnPath,
}: {
  loginUrl?: string
  returnPath?: string
}) {
  const href = returnPath ? `${loginUrl}?return=${encodeURIComponent(returnPath)}` : loginUrl
  return (
    <HostedPanel
      title="Sign in to Fathom"
      actions={
        <Button asChild>
          <a href={href}>Sign in</a>
        </Button>
      }
    >
      <p>You need to sign in with your organization account to continue.</p>
    </HostedPanel>
  )
}
