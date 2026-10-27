import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { App } from '@/app/App'
import { useSession } from '@/app/session-store'
import type { FetchLike } from '@/protocol/http'

import { FakeSocket } from './fake-socket'
import { env, frames, ME, resetSession, signIn, timers } from './session-helpers'

beforeEach(resetSession)

describe('session bootstrap', () => {
  it('is signed out without the CSRF cookie and opens no socket', () => {
    useSession.getState().bootstrap({ cookies: 'other=1', transportEnv: env })
    expect(useSession.getState().status).toBe('signed_out')
    expect(FakeSocket.instances).toHaveLength(0)
  })

  it('says Hello with the cookie token, asks GetMe, and is ready on Me', () => {
    useSession.getState().bootstrap({ cookies: 'keel_csrf=tok-1', transportEnv: env })
    expect(useSession.getState().status).toBe('loading')
    const s = FakeSocket.last()
    s.open()
    const [hello, getMe] = frames(s)
    expect(hello?.csrf_token).toBe('tok-1')
    expect(getMe?.cmd).toEqual({ type: 'GetMe' })
    s.serverSend({ reply_to: getMe?.id, event: { type: 'Me', data: ME } })
    expect(useSession.getState()).toMatchObject({ status: 'ready', me: ME, connection: 'open' })
  })

  it('returns to the page remembered before sign-in', () => {
    sessionStorage.setItem('keel:return-to', '/audit')
    signIn()
    expect(location.pathname).toBe('/audit')
    expect(sessionStorage.getItem('keel:return-to')).toBeNull()
  })

  it('ignores a remembered path that leaves the origin', () => {
    sessionStorage.setItem('keel:return-to', '//evil.example.com/x')
    signIn()
    expect(location.pathname).toBe('/')
  })

  it('updates verification from a pushed Verification', () => {
    const s = signIn()
    const v = { ...ME.verification, verified: true, enrolled: true, at: '2026-10-27T10:00:00Z' }
    s.serverSend({ event: { type: 'Verification', data: v } })
    expect(useSession.getState().me?.verification).toEqual(v)
  })

  it('asks GetMe again after a reconnect', () => {
    signIn().serverClose(1006)
    expect(useSession.getState().connection).toBe('reconnecting')
    timers.shift()?.()
    const again = FakeSocket.last()
    again.open()
    expect(frames(again).filter((f) => f.cmd?.type === 'GetMe')).toHaveLength(1)
    expect(useSession.getState().connection).toBe('open')
  })
})

describe('session endings', () => {
  it('Revoked signs out with the revoked notice', () => {
    const s = signIn()
    s.serverSend({ event: { type: 'Revoked' } })
    expect(useSession.getState()).toMatchObject({ status: 'signed_out', notice: 'revoked', me: null })
  })

  it('4401 after sign-in signs out as expired; before it, with no notice', () => {
    signIn().serverClose(4401)
    expect(useSession.getState()).toMatchObject({ status: 'signed_out', notice: 'expired' })

    useSession.getState().bootstrap({ cookies: 'keel_csrf=stale', transportEnv: env })
    FakeSocket.last().serverClose(4401)
    expect(useSession.getState()).toMatchObject({ status: 'signed_out', notice: null })
  })

  it('4400 shows the outdated notice', () => {
    signIn().serverClose(4400)
    expect(useSession.getState().status).toBe('outdated')
  })

  it('an OpError answering GetMe is an error', () => {
    useSession.getState().bootstrap({ cookies: 'keel_csrf=tok-1', transportEnv: env })
    const s = FakeSocket.last()
    s.open()
    s.serverSend({ reply_to: 1, event: { type: 'OpError', data: { code: 'internal', message: 'db down' } } })
    expect(useSession.getState().status).toBe('error')
  })

  it('sign-out posts the CSRF token and closes the socket', async () => {
    const s = signIn()
    const fetchFn = vi.fn<FetchLike>(() => Promise.resolve(new Response(JSON.stringify({ endSessionUrl: null }))))
    await useSession.getState().signOut(fetchFn)
    const [path, init] = fetchFn.mock.calls[0] ?? []
    expect(path).toBe('/auth/logout')
    expect(new Headers(init?.headers).get('X-CSRF-Token')).toBe('tok-1')
    expect(s.readyState).toBe(3)
    expect(useSession.getState()).toMatchObject({ status: 'signed_out', notice: null })
  })
})

describe('App', () => {
  it('shows the sign-in page without a session', async () => {
    render(<App />)
    expect(await screen.findByRole('heading', { name: 'Sign in' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Sign in' })).toHaveAttribute('href', '/auth/login')
  })
})
