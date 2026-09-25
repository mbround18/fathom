import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { LimitReached } from '@/components/hosted/LimitReached'
import { NoAccess } from '@/components/hosted/NoAccess'
import { SessionExpired } from '@/components/hosted/SessionExpired'
import { applySubscriptionInfo } from '@/store/handlers/hosted'

describe('hosted components', () => {
  it('renders session-ended copy per reason', () => {
    render(<SessionExpired reason="revoked" />)
    expect(screen.getByRole('heading', { name: 'Access revoked' })).toBeTruthy()
  })
  it('renders no-access with the username', () => {
    render(<NoAccess username="carol" />)
    expect(screen.getByText(/carol is signed in/)).toBeTruthy()
  })
  it('renders the limit text', () => {
    render(<LimitReached kind="tabs" max={3} />)
    expect(screen.getByText(/at most 3 browser tabs/)).toBeTruthy()
  })
})

describe('applySubscriptionInfo', () => {
  it('is idempotent and keeps terminal state', () => {
    const info = {
      protocolVersion: 1 as const,
      backendStarted: true,
      mode: 'hosted' as const,
      identity: { username: 'a', groups: [] },
    }
    const once = applySubscriptionInfo({ mode: 'desktop' }, info)
    const ended = { ...once, sessionEnded: 'expired' as const }
    expect(applySubscriptionInfo(ended, info)).toEqual(ended)
    expect(
      applySubscriptionInfo({ mode: 'hosted' }, { protocolVersion: 1, backendStarted: true }).mode,
    ).toBe('desktop')
  })
})
