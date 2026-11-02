import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { isTransportFailure, RefusalText, type Refusal } from '../../src/kit'

const OFFLINE: Refusal = { kind: 'error', code: 'offline', message: 'Not connected.' }

describe('isTransportFailure', () => {
  it('is true only for the offline error', () => {
    expect(isTransportFailure(OFFLINE)).toBe(true)
    expect(isTransportFailure({ kind: 'error', code: 'internal', message: 'Down.' })).toBe(false)
    expect(isTransportFailure({ kind: 'denied', action: 'offline', reason: 'offline' })).toBe(false)
  })
})

describe('RefusalText', () => {
  it('shows a denial as its action and reason code', () => {
    render(<RefusalText refusal={{ kind: 'denied', action: 'delete_widget', reason: 'no_grant' }} />)
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent('Denied: delete_widget (no_grant)')
    expect(alert).toHaveAttribute('data-refusal', 'denied')
    expect(alert.querySelector('[data-denied-action]')).toHaveTextContent('delete_widget')
    expect(alert.querySelector('[data-denied-reason]')).toHaveTextContent('no_grant')
  })

  it('shows a denial without an action as its reason code alone', () => {
    render(<RefusalText refusal={{ kind: 'denied', reason: 'not_permitted' }} />)
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent(/^Denied \(not_permitted\)$/)
    expect(alert.querySelector('[data-denied-action]')).toBeNull()
    expect(alert.querySelector('[data-denied-reason]')).toHaveTextContent('not_permitted')
  })

  it('shows an error’s message and code', () => {
    render(<RefusalText refusal={{ kind: 'error', code: 'conflict', message: 'The widget changed.' }} />)
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent('The widget changed. (conflict)')
    expect(alert.querySelector('[data-error-code]')).toHaveTextContent('conflict')
  })

  it('shows offline without a code', () => {
    render(<RefusalText refusal={OFFLINE} />)
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent(/^Not connected\.$/)
    expect(alert.querySelector('[data-error-code]')).toBeNull()
  })

  it('puts the lead first, in the same alert', () => {
    render(<RefusalText refusal={OFFLINE} lead="Could not save." />)
    expect(screen.getByRole('alert')).toHaveTextContent('Could not save. Not connected.')
  })
})
