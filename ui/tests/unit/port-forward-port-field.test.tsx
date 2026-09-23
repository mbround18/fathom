/**
 * PortForwardPortField (T008, 009 US1/US3): the debounced port-availability check dispatches
 * `CheckPortAvailability` only after edits settle for 300ms (data-model.md), not on every
 * keystroke.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { PortForwardPortField } from '@/components/panels/PortForwardPortField'
import { initialData, useStore } from '@/store'

beforeEach(() => {
  useStore.setState(initialData())
  vi.useFakeTimers()
})

function renderField(value = 8080) {
  const send = vi.fn()
  useStore.setState({ send })
  const onChange = vi.fn()
  render(<PortForwardPortField id="p" value={value} onChange={onChange} />)
  return { send, onChange }
}

describe('PortForwardPortField', () => {
  it('does not dispatch CheckPortAvailability on every keystroke', () => {
    const { send } = renderField()
    const input = screen.getByTestId('port-forward-local-port')
    fireEvent.change(input, { target: { value: '9' } })
    fireEvent.change(input, { target: { value: '90' } })
    fireEvent.change(input, { target: { value: '900' } })
    expect(send).not.toHaveBeenCalled()
  })

  it('dispatches CheckPortAvailability once the 300ms debounce settles', () => {
    const { send } = renderField()
    const input = screen.getByTestId('port-forward-local-port')
    fireEvent.change(input, { target: { value: '9090' } })
    vi.advanceTimersByTime(299)
    expect(send).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(send).toHaveBeenCalledWith({ type: 'CheckPortAvailability', data: { port: 9090 } })
    expect(send).toHaveBeenCalledTimes(1)
  })

  it('resets the debounce timer on each edit rather than firing per keystroke', () => {
    const { send } = renderField()
    const input = screen.getByTestId('port-forward-local-port')
    fireEvent.change(input, { target: { value: '9' } })
    vi.advanceTimersByTime(200)
    fireEvent.change(input, { target: { value: '90' } })
    vi.advanceTimersByTime(200)
    expect(send).not.toHaveBeenCalled()
    vi.advanceTimersByTime(100)
    expect(send).toHaveBeenCalledTimes(1)
    expect(send).toHaveBeenCalledWith({ type: 'CheckPortAvailability', data: { port: 90 } })
  })

  it('renders a conflict warning when the backend reports the port unavailable', () => {
    useStore.setState({ send: vi.fn() })
    useStore.setState((s) => ({
      integrations: { ...s.integrations, portAvailability: { 8080: false } },
    }))
    render(<PortForwardPortField id="p" value={8080} onChange={vi.fn()} />)
    expect(screen.getByTestId('port-forward-port-conflict')).toBeTruthy()
  })
})
