import { act, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { App } from '@/app/App'
import { disconnect } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { Me } from '@/protocol/generated'
import { PROTOCOL_VERSION } from '@/protocol/generated/constants'

import { FakeSocket } from './fake-socket'

function me(over: Partial<Me> = {}): Me {
  return {
    identity: { provider: 'github', subject: '42', username: 'ada', email: null },
    csrf_token: 'tok-1',
    role: 'user',
    owner: false,
    stacks: ['web'],
    features: { shell: true, edit: true, host_metrics: false },
    network: { trusted: true, reason: 'first', network: '203.0.113.7/32' },
    needs_answer: false,
    proxy_warning: false,
    ...over,
  }
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

let meResponses: Response[]
const fetchMock = vi.fn<(input: string, init?: RequestInit) => Promise<Response>>((input) => {
  if (input === '/api/v1/me') {
    return Promise.resolve(meResponses.shift() ?? json(401, { error: 'unauthenticated' }))
  }
  return Promise.resolve(json(200, {}))
})

beforeEach(() => {
  FakeSocket.reset()
  meResponses = []
  fetchMock.mockClear()
  vi.stubGlobal('fetch', fetchMock)
  vi.stubGlobal('WebSocket', FakeSocket)
  useSession.setState({ status: 'loading', me: null, providers: [], connection: 'closed' })
})

afterEach(() => {
  disconnect()
  vi.unstubAllGlobals()
})

describe('App bootstrap', () => {
  it('shows sign-in with the configured providers on a 401, linking to /auth/login', async () => {
    meResponses = [json(401, { error: 'unauthenticated', providers: ['github', 'discord', 'evil'] })]
    render(<App />)
    const gh = await screen.findByRole('link', { name: 'Sign in with GitHub' })
    expect(gh).toHaveAttribute('href', '/auth/login?provider=github')
    expect(screen.getByRole('link', { name: 'Sign in with Discord' })).toHaveAttribute(
      'href',
      '/auth/login?provider=discord',
    )
    expect(screen.getAllByRole('link')).toHaveLength(2)
    expect(FakeSocket.instances).toHaveLength(0)
  })

  it('offers a plain sign-in link when the 401 names no providers', async () => {
    render(<App />)
    expect(await screen.findByRole('link', { name: 'Sign in' })).toHaveAttribute(
      'href',
      '/auth/login',
    )
  })

  it.each([
    ['pending', me({ role: 'pending', stacks: [] })],
    ['a user with no stacks', me({ role: 'user', stacks: [] })],
  ])('shows the no-access page for %s, without opening a socket', async (_, who) => {
    meResponses = [json(200, who)]
    render(<App />)
    expect(
      await screen.findByRole('heading', { name: "You don't currently have access" }),
    ).toBeInTheDocument()
    expect(screen.queryByText('web')).toBeNull()
    expect(FakeSocket.instances).toHaveLength(0)
  })

  it('opens the shell for a user with stacks and sends Hello with the CSRF token', async () => {
    meResponses = [json(200, me())]
    render(<App />)
    expect(await screen.findByRole('heading', { name: 'Stacks' })).toBeInTheDocument()
    const s = FakeSocket.last()
    expect(s.url).toBe(`ws://${location.host}/api/v1/ws`)
    act(() => s.open())
    expect(s.frames()[0]).toEqual({ protocol_version: PROTOCOL_VERSION, csrf_token: 'tok-1' })
  })

  it('routes a 4401 close to the sign-in page', async () => {
    meResponses = [json(200, me()), json(401, { error: 'unauthenticated', providers: ['oidc'] })]
    render(<App />)
    await screen.findByRole('heading', { name: 'Stacks' })
    act(() => FakeSocket.last().open())
    act(() => FakeSocket.last().serverClose(4401))
    expect(screen.getByRole('heading', { name: 'Sign in' })).toBeInTheDocument()
    expect(
      await screen.findByRole('link', { name: 'Sign in with your identity provider' }),
    ).toHaveAttribute('href', '/auth/login?provider=oidc')
    expect(FakeSocket.instances).toHaveLength(1)
  })

  it('routes a 4403 close to the no-access page', async () => {
    meResponses = [json(200, me())]
    render(<App />)
    await screen.findByRole('heading', { name: 'Stacks' })
    act(() => FakeSocket.last().open())
    act(() => FakeSocket.last().serverClose(4403))
    expect(
      await screen.findByRole('heading', { name: "You don't currently have access" }),
    ).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Stacks' })).toBeNull()
    expect(FakeSocket.instances).toHaveLength(1)
  })

  it('sends X-CSRF-Token on sign-out', async () => {
    meResponses = [json(200, me())]
    render(<App />)
    await screen.findByRole('heading', { name: 'Stacks' })
    await act(() => useSession.getState().signOut())
    const call = fetchMock.mock.calls.find(([u]) => u === '/auth/logout')
    expect(call).toBeDefined()
    const headers = new Headers(call?.[1]?.headers)
    expect(headers.get('X-CSRF-Token')).toBe('tok-1')
    expect(call?.[1]?.method).toBe('POST')
    expect(await screen.findByRole('heading', { name: 'Sign in' })).toBeInTheDocument()
  })
})
