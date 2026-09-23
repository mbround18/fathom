/**
 * 011-rbac-oidc-administration (T019): `DegradedScopeBanner` renders the affected scopes and the
 * backend's error text verbatim (constitution I: never paraphrase). Verified as a component in
 * isolation here rather than through the US1 e2e spec — see that spec's header comment for why.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { DegradedScopeBanner } from '@/components/rbac/DegradedScopeBanner'

describe('DegradedScopeBanner', () => {
  it('renders nothing when there are no degraded scopes and no message', () => {
    const { container } = render(<DegradedScopeBanner scopes={[]} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('lists the affected scopes', () => {
    render(<DegradedScopeBanner scopes={['kube-system', 'payments']} />)
    const banner = screen.getByTestId('rbac-degraded-scope')
    expect(banner).toHaveTextContent('kube-system')
    expect(banner).toHaveTextContent('payments')
  })

  it('shows the backend error message verbatim, unmodified', () => {
    const verbatim =
      'roles.rbac.authorization.k8s.io is forbidden: User "jane" cannot list resource'
    render(<DegradedScopeBanner scopes={['kube-system']} message={verbatim} />)
    expect(screen.getByTestId('rbac-degraded-scope')).toHaveTextContent(verbatim)
  })

  it('still renders with a message even when scopes is empty', () => {
    render(<DegradedScopeBanner scopes={[]} message="partial failure" />)
    expect(screen.getByTestId('rbac-degraded-scope')).toHaveTextContent('partial failure')
  })
})
