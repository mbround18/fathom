/**
 * 012-incluster-daemon-federation P1 (T014): `ContextStatusUpdated` handler coverage
 * (`ui/src/store/handlers/system.ts`), following the reducer-test pattern in `ingest.test.ts`.
 */
import { beforeEach, describe, expect, it } from 'vitest'

import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'
import { ingest } from '@/store/ingest'

import { federationContexts, federationStatuses, FEDERATION_ERROR_MESSAGE } from '../fixtures'
import { batch, system } from '../fixtures/builders'

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
})

describe('ContextStatusUpdated (federation-context-status)', () => {
  it('records status per context, error only on Error status', () => {
    ingest(federationContexts)
    ingest(federationStatuses)
    const { contextStatuses } = useStore.getState().session
    expect(contextStatuses['kind-a']).toEqual({ status: 'Connected' })
    expect(contextStatuses['kind-b']).toEqual({ status: 'NotConnected' })
    expect(contextStatuses['kind-c']).toEqual({
      status: 'Error',
      error: FEDERATION_ERROR_MESSAGE,
    })
  })

  it('a context absent from contextStatuses has no entry (renders as not-connected by convention)', () => {
    ingest(federationContexts)
    expect(useStore.getState().session.contextStatuses).toEqual({})
  })

  it('last write wins for a context whose status changes (Connecting -> Connected)', () => {
    ingest(federationContexts)
    ingest(
      batch([
        system(5, {
          type: 'ContextStatusUpdated',
          data: { context: 'kind-a', status: 'Connecting', error: null, plugin_error: null },
        }),
      ]),
    )
    expect(useStore.getState().session.contextStatuses['kind-a']).toEqual({ status: 'Connecting' })
    ingest(
      batch([
        system(6, {
          type: 'ContextStatusUpdated',
          data: { context: 'kind-a', status: 'Connected', error: null, plugin_error: null },
        }),
      ]),
    )
    expect(useStore.getState().session.contextStatuses['kind-a']).toEqual({ status: 'Connected' })
  })

  it('a later Error clears a prior status entry by overwriting it, carrying the new message', () => {
    ingest(federationContexts)
    ingest(
      batch([
        system(5, {
          type: 'ContextStatusUpdated',
          data: { context: 'kind-b', status: 'NotConnected', error: null, plugin_error: null },
        }),
      ]),
    )
    ingest(
      batch([
        system(6, {
          type: 'ContextStatusUpdated',
          data: {
            context: 'kind-b',
            status: 'Error',
            error: 'connection refused',
            plugin_error: null,
          },
        }),
      ]),
    )
    expect(useStore.getState().session.contextStatuses['kind-b']).toEqual({
      status: 'Error',
      error: 'connection refused',
    })
  })

  it('leaves other contexts and session fields untouched', () => {
    ingest(federationContexts)
    ingest(federationStatuses)
    const { session } = useStore.getState()
    expect(session.activeContext).toBe('kind-a')
    expect(session.contexts).toEqual(['kind-a', 'kind-b', 'kind-c'])
  })
})
