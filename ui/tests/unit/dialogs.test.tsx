/** ConfirmDialog (T092) and ScaleDialog (T095): naming, typed confirm, Esc/Cancel send nothing. */
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { parseReplicas } from '@/components/action-utils'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ScaleDialog } from '@/components/ScaleDialog'

const items = (n: number) =>
  Array.from({ length: n }, (_, i) => ({ namespace: 'default', name: `api-${String(i)}` }))

function confirm(n: number) {
  const onConfirm = vi.fn()
  const onCancel = vi.fn()
  render(
    <ConfirmDialog
      open
      title={`Delete ${String(n)} pods?`}
      items={items(n)}
      destructive
      confirmLabel={`Delete ${String(n)} pods`}
      onConfirm={onConfirm}
      onCancel={onCancel}
    />,
  )
  return { onConfirm, onCancel }
}

describe('ConfirmDialog', () => {
  it('names every object by namespace/name', () => {
    confirm(3)
    const names = screen.getAllByTestId('confirm-item').map((li) => li.textContent)
    expect(names).toEqual(['✖default/api-0', '✖default/api-1', '✖default/api-2'])
    expect(screen.getByTestId('confirm-typed-count')).toBeTruthy()
  })

  it('confirms with the button; Cancel and Esc send nothing', async () => {
    const user = userEvent.setup()
    const { onConfirm, onCancel } = confirm(1)
    await user.click(screen.getByTestId('confirm-cancel'))
    expect(onCancel).toHaveBeenCalledTimes(1)
    await user.keyboard('{Escape}')
    expect(onConfirm).not.toHaveBeenCalled()
    await user.click(screen.getByTestId('confirm-submit'))
    expect(onConfirm).toHaveBeenCalledTimes(1)
  })

  it('more than one object: the count must be typed first', async () => {
    const user = userEvent.setup()
    const { onConfirm } = confirm(6)
    const submit = screen.getByTestId('confirm-submit')
    expect(submit).toBeDisabled()
    const input = screen.getByLabelText('Type 6 to confirm')
    await user.type(input, '5')
    expect(submit).toBeDisabled()
    await user.clear(input)
    await user.type(input, '6{Enter}')
    expect(onConfirm).toHaveBeenCalledTimes(1)
  })

  it('a single object needs no typing: its manifest is one readable line', () => {
    confirm(1)
    expect(screen.queryByTestId('confirm-typed-count')).toBeNull()
    expect(screen.getByTestId('confirm-submit')).toBeEnabled()
  })

  it('two objects already require the count, so no bulk delete arms on first paint', () => {
    confirm(2)
    expect(screen.getByTestId('confirm-typed-count')).toBeTruthy()
    expect(screen.getByTestId('confirm-submit')).toBeDisabled()
  })
})

describe('ScaleDialog', () => {
  function scale(current = 1) {
    const onConfirm = vi.fn()
    const onCancel = vi.fn()
    render(
      <ScaleDialog
        open
        kind="Deployment"
        item={{ namespace: 'default', name: 'api' }}
        current={current}
        onConfirm={onConfirm}
        onCancel={onCancel}
      />,
    )
    return { onConfirm, onCancel }
  }

  it('shows current → new and confirms the typed count', async () => {
    const user = userEvent.setup()
    const { onConfirm } = scale(1)
    expect(screen.getByTestId('scale-target')).toHaveTextContent('default/api')
    const input = screen.getByLabelText('Replicas')
    expect(screen.getByTestId('scale-submit')).toBeDisabled()
    await user.clear(input)
    await user.type(input, '3')
    expect(screen.getByTestId('scale-current')).toHaveTextContent('1')
    expect(screen.getByTestId('scale-new')).toHaveTextContent('3')
    expect(screen.getByTestId('scale-submit')).toHaveTextContent('Scale 1 → 3')
    await user.click(screen.getByTestId('scale-submit'))
    expect(onConfirm).toHaveBeenCalledWith(3)
  })

  it('steppers adjust the count; invalid input disables submit', async () => {
    const user = userEvent.setup()
    scale(2)
    await user.click(screen.getByTestId('scale-increment'))
    expect(screen.getByTestId('scale-new')).toHaveTextContent('3')
    const input = screen.getByLabelText('Replicas')
    await user.clear(input)
    expect(screen.getByTestId('scale-submit')).toBeDisabled()
  })

  it('Cancel sends nothing', async () => {
    const user = userEvent.setup()
    const { onConfirm, onCancel } = scale()
    await user.click(screen.getByTestId('scale-cancel'))
    expect(onCancel).toHaveBeenCalled()
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('parseReplicas accepts whole numbers 0..1000 only', () => {
    expect(parseReplicas('0')).toBe(0)
    expect(parseReplicas(' 12 ')).toBe(12)
    expect(parseReplicas('1001')).toBeUndefined()
    expect(parseReplicas('-1')).toBeUndefined()
    expect(parseReplicas('1.5')).toBeUndefined()
    expect(parseReplicas('')).toBeUndefined()
  })
})
