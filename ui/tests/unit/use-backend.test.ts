/** connectBackend + FakeTransport: starting → ready, bootstrap commands, backend-gone. */
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { FakeTransport } from '@/protocol/fake-transport'
import { connectBackend } from '@/protocol/use-backend'
import { initialData, useStore } from '@/store'

beforeEach(() => {
  useStore.setState(initialData())
  vi.useFakeTimers()
})

describe('connectBackend', () => {
  it('reaches ready, sends the bootstrap commands, and loads the fixture contexts', async () => {
    const fake = new FakeTransport()
    await connectBackend(fake)
    expect(useStore.getState().conn.status).toBe('ready')
    expect(fake.sent().map((c) => c.type)).toEqual([
      'LoadUiConfig',
      'RefreshContexts',
      'Initialize',
    ])
    await vi.runAllTimersAsync()
    const { session } = useStore.getState()
    expect(session.contexts).toEqual(['kind-a', 'kind-b'])
    expect(session.activeContext).toBe('kind-a')
    expect(session.namespaces.length).toBeGreaterThan(0)
  })

  it('selectContext sends SetContext and the fake switches the session', async () => {
    const fake = new FakeTransport()
    await connectBackend(fake)
    await vi.runAllTimersAsync()
    fake.clearSent()
    useStore.getState().selectContext('kind-b')
    expect(fake.sent()).toEqual([{ type: 'SetContext', data: { context: 'kind-b' } }])
    await vi.runAllTimersAsync()
    expect(useStore.getState().session.activeContext).toBe('kind-b')
  })

  it('a rejected send moves the connection to backend-gone', async () => {
    const fake = new FakeTransport()
    await connectBackend(fake)
    fake.setBackendGone(true)
    useStore.getState().send({ type: 'RefreshPods' })
    await vi.runAllTimersAsync()
    const { conn } = useStore.getState()
    expect(conn.status).toBe('backend-gone')
    expect(conn.fatal).toMatch(/BackendGone/)
  })

  it('a rejected subscribe is backend-gone from the start', async () => {
    const fake = new FakeTransport()
    fake.setBackendGone(true)
    await connectBackend(fake)
    expect(useStore.getState().conn.status).toBe('backend-gone')
    expect(fake.sent()).toEqual([])
  })
})
