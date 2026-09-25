import { useStore } from '@/store'
import { signOutHosted } from '@/protocol/sign-out'

import { LimitReached } from './LimitReached'
import { NoAccess } from './NoAccess'
import { SessionExpired } from './SessionExpired'

/**
 * Hosted-mode terminal states in front of the shell (031): session ended, then zero-permission
 * user. Limit notices are overlaid by `HostedLimitNotice`. Desktop renders children untouched.
 */
export function HostedGate({ children }: { children: React.ReactNode }) {
  const hosted = useStore((s) => s.hosted)
  const contextsLoaded = useStore((s) => s.session.contextsLoaded)
  const contexts = useStore((s) => s.session.contexts)
  const notice = useStore((s) => s.conn.lastNotice)
  if (hosted.mode !== 'hosted') return <>{children}</>
  if (hosted.sessionEnded) return <SessionExpired reason={hosted.sessionEnded} />
  if (contextsLoaded && contexts.length === 0) {
    return (
      <NoAccess
        username={hosted.identity?.username ?? 'You'}
        {...(notice?.level === 'error' && { message: notice.message })}
        onSignOut={() => void signOutHosted()}
      />
    )
  }
  return (
    <>
      {children}
      {hosted.limitReached && (
        <div className="fixed right-2 bottom-2 z-50 max-w-sm" data-testid="limit-reached">
          <LimitReached
            kind={hosted.limitReached.kind}
            max={hosted.limitReached.max}
            onDismiss={() => {
              const next = { ...useStore.getState().hosted }
              delete next.limitReached
              useStore.setState({ hosted: next })
            }}
          />
        </div>
      )}
    </>
  )
}
