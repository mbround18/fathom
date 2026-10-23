import type { SessionEndReason } from '@/protocol/generated/SessionEndReason'
import { Button } from '@companion/ui'
import { HostedPanel } from './HostedPanel'

const COPY: Record<SessionEndReason, { title: string; body: string }> = {
  signedOut: { title: 'Signed out', body: 'You have been signed out.' },
  expired: {
    title: 'Session expired',
    body: 'Your session expired. Sign in again to continue where you left off.',
  },
  revoked: {
    title: 'Access revoked',
    body: 'Your session was ended by an administrator or your identity provider.',
  },
  idpUnavailable: {
    title: 'Identity provider unavailable',
    body: 'Fathom could not refresh your session because the identity provider is unreachable. Try again shortly.',
  },
  limitEvicted: {
    title: 'Session replaced',
    body: 'This session was ended because you reached your concurrent session limit.',
  },
}

export function SessionExpired({
  reason,
  loginUrl = '/auth/login',
}: {
  reason: SessionEndReason
  loginUrl?: string
}) {
  const { title, body } = COPY[reason]
  return (
    <HostedPanel
      title={title}
      role="alert"
      actions={
        <Button asChild>
          <a href={loginUrl}>Sign in again</a>
        </Button>
      }
    >
      <p>{body}</p>
    </HostedPanel>
  )
}
