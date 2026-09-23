/**
 * Origin-tagging merge logic (T009, 009 data-model.md): a pending Service-launched target is
 * matched against the next `PortForwardsUpdated` batch by namespace/pod/local_port/remote_port;
 * exactly the first matching, untagged row gets tagged and the pending entry is consumed once.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'

// `@/store` must be imported before any descriptor module (see tests/unit/ingest.test.ts).
import { initialData, useStore } from '@/store'
import { ingest } from '@/store/ingest'
import type { PortForward } from '@/protocol/types'

import { batch, list, nsRoute } from '../fixtures/builders'

vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), loading: vi.fn() } }))

const CTX = 'kind-a'
const ROUTE = nsRoute(CTX, 'network/portforwards')

function pf(overrides: Partial<PortForward>): PortForward {
  return {
    id: 'pf-1',
    namespace: 'default',
    pod: 'web-0',
    local_port: 8080,
    remote_port: 80,
    container: null,
    status: 'Connecting',
    auto_start: false,
    ...overrides,
  }
}

beforeEach(() => {
  useStore.setState(initialData())
  useStore.setState((s) => ({ session: { ...s.session, activeContext: CTX } }))
})

describe('port-forward origin tagging', () => {
  it('tags exactly the first matching untagged row for a pending Service-launched start', () => {
    useStore.getState().recordPortForwardOrigin({
      namespace: 'default',
      pod: 'web-0',
      localPort: 8080,
      remotePort: 80,
      service: 'web',
    })
    expect(Object.keys(useStore.getState().pendingPortForwardOrigins)).toHaveLength(1)

    ingest(
      batch([
        list(1, ROUTE, {
          type: 'PortForwardsUpdated',
          data: [pf({ id: 'pf-1' }), pf({ id: 'pf-2', pod: 'web-1' })],
        }),
      ]),
    )

    const { portForwardOrigins, pendingPortForwardOrigins } = useStore.getState()
    expect(portForwardOrigins).toEqual({ 'pf-1': { service: 'web' } })
    expect(portForwardOrigins['pf-2']).toBeUndefined()
    // The pending entry is consumed at most once.
    expect(Object.keys(pendingPortForwardOrigins)).toHaveLength(0)
  })

  it('leaves non-matching rows untouched', () => {
    useStore.getState().recordPortForwardOrigin({
      namespace: 'default',
      pod: 'web-0',
      localPort: 8080,
      remotePort: 80,
      service: 'web',
    })

    ingest(
      batch([
        list(1, ROUTE, {
          type: 'PortForwardsUpdated',
          data: [pf({ id: 'pf-2', pod: 'other', local_port: 9090, remote_port: 90 })],
        }),
      ]),
    )

    const { portForwardOrigins, pendingPortForwardOrigins } = useStore.getState()
    expect(portForwardOrigins).toEqual({})
    expect(Object.keys(pendingPortForwardOrigins)).toHaveLength(1)
  })

  it('does not re-tag a row that already has a recorded origin', () => {
    useStore.setState({ portForwardOrigins: { 'pf-1': { service: 'other-svc' } } })
    useStore.getState().recordPortForwardOrigin({
      namespace: 'default',
      pod: 'web-0',
      localPort: 8080,
      remotePort: 80,
      service: 'web',
    })

    ingest(
      batch([
        list(1, ROUTE, {
          type: 'PortForwardsUpdated',
          data: [pf({ id: 'pf-1' })],
        }),
      ]),
    )

    const { portForwardOrigins, pendingPortForwardOrigins } = useStore.getState()
    expect(portForwardOrigins['pf-1']).toEqual({ service: 'other-svc' })
    // Still pending: the only existing row with this identity is already tagged.
    expect(Object.keys(pendingPortForwardOrigins)).toHaveLength(1)
  })

  it('is a no-op when there is no pending origin', () => {
    ingest(
      batch([
        list(1, ROUTE, {
          type: 'PortForwardsUpdated',
          data: [pf({ id: 'pf-1' })],
        }),
      ]),
    )
    expect(useStore.getState().portForwardOrigins).toEqual({})
  })
})
