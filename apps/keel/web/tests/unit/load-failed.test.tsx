import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { loadList, OFFLINE, type Refusal } from '@/app/load'
import type { Event } from '@/protocol/wire'
import { LoadFailed } from '@/ui/LoadFailed'

import { lastCmd, resetSession, signIn } from './session-helpers'

// A list that fails to load says so, and why, instead of loading forever.

const listRuns = {
  type: 'ListRuns',
  data: { workspace: 'w-1', binding: null, host: null, actor: null, state: null, from: null, to: null },
} as const
const pick = (event: Event) => (event.type === 'Runs' ? event.data.items : undefined)

beforeEach(() => {
  resetSession()
})

describe('loadList', () => {
  it('hands the answer to loaded, once', () => {
    const s = signIn()
    const on = { loaded: vi.fn(), failed: vi.fn() }
    loadList(listRuns, pick, on)
    const id = lastCmd(s, 'ListRuns')?.id
    s.serverSend({ reply_to: id, event: { type: 'Runs', data: { items: [] } } })
    s.serverSend({ reply_to: id, event: { type: 'Runs', data: { items: [] } } })
    expect(on.loaded).toHaveBeenCalledTimes(1)
    expect(on.loaded).toHaveBeenCalledWith([])
    expect(on.failed).not.toHaveBeenCalled()
  })

  it('keeps an OpError or Denied answer as the server sent it', () => {
    const s = signIn()
    const err = { loaded: vi.fn(), failed: vi.fn() }
    loadList(listRuns, pick, err)
    s.serverSend({
      reply_to: lastCmd(s, 'ListRuns')?.id,
      event: { type: 'OpError', data: { code: 'internal', message: 'The run store is down.' } },
    })
    expect(err.failed).toHaveBeenCalledWith({ kind: 'error', code: 'internal', message: 'The run store is down.' })

    const denied = { loaded: vi.fn(), failed: vi.fn() }
    loadList(listRuns, pick, denied)
    s.serverSend({
      reply_to: lastCmd(s, 'ListRuns')?.id,
      event: { type: 'Denied', data: { action: 'list_runs', reason: 'no_grant' } },
    })
    expect(denied.failed).toHaveBeenCalledWith({ kind: 'denied', action: 'list_runs', reason: 'no_grant' })
    expect(err.loaded).not.toHaveBeenCalled()
    expect(denied.loaded).not.toHaveBeenCalled()
  })

  it('ignores other replies on the same request', () => {
    const s = signIn()
    const on = { loaded: vi.fn(), failed: vi.fn() }
    loadList(listRuns, pick, on)
    s.serverSend({ reply_to: lastCmd(s, 'ListRuns')?.id, event: { type: 'Hosts', data: { items: [] } } })
    expect(on.loaded).not.toHaveBeenCalled()
    expect(on.failed).not.toHaveBeenCalled()
  })

  it('fails straight away as OFFLINE with no connection', () => {
    const on = { loaded: vi.fn(), failed: vi.fn() }
    loadList(listRuns, pick, on)
    expect(on.failed).toHaveBeenCalledWith(OFFLINE)
  })
})

describe('LoadFailed', () => {
  const opError: Refusal = { kind: 'error', code: 'internal', message: 'The run store is down.' }

  it('says what failed and why, as an error pane', () => {
    const { container } = render(<LoadFailed what="runs" refusal={opError} />)
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent('Failed to load runs. The run store is down. (internal)')
    expect(container.querySelector('[data-pane-state="error"]')).not.toBeNull()
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull()
  })

  it('shows a denial with its action and reason', () => {
    render(<LoadFailed what="hosts" refusal={{ kind: 'denied', action: 'list_hosts', reason: 'no_grant' }} />)
    expect(screen.getByRole('alert')).toHaveTextContent('Failed to load hosts. Denied: list_hosts (no_grant)')
  })

  it('gives a plain reason when Keel is not connected', () => {
    render(<LoadFailed what="bindings" refusal={OFFLINE} />)
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent(`Failed to load bindings. ${OFFLINE.kind === 'error' ? OFFLINE.message : ''}`)
    expect(alert.querySelector('[data-error-code]')).toBeNull()
  })

  it('runs the load again on Retry', async () => {
    const retry = vi.fn()
    render(<LoadFailed what="runs" refusal={opError} onRetry={retry} />)
    await userEvent.click(screen.getByRole('button', { name: 'Retry' }))
    expect(retry).toHaveBeenCalledTimes(1)
  })
})
