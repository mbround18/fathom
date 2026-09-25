/** DebugContainerDialog (025, US3): summary + irreversibility warning, gated confirm. */
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { EPHEMERAL_WARNING } from '@/components/action-utils'
import { DebugContainerDialog } from '@/components/DebugContainerDialog'
import { addEphemeralContainerAction } from '@/resources/actions'

const item = { namespace: 'default', name: 'web-0' }

function setup(containers: string[]) {
  const onConfirm = vi.fn()
  const onCancel = vi.fn()
  render(
    <DebugContainerDialog
      open
      item={item}
      containers={containers}
      onConfirm={onConfirm}
      onCancel={onCancel}
    />,
  )
  return { onConfirm, onCancel }
}

describe('DebugContainerDialog', () => {
  it('shows pod, target, image and the exact warning; confirm is gated on image', async () => {
    const user = userEvent.setup()
    const { onConfirm } = setup(['app'])
    expect(screen.getByTestId('debug-target-pod')).toHaveTextContent('default/web-0')
    expect(screen.getByTestId('debug-warning')).toHaveTextContent(EPHEMERAL_WARNING)
    expect(screen.getByTestId('debug-target-container')).toHaveValue('app')
    expect(screen.getByTestId('debug-submit')).toBeDisabled()

    await user.type(screen.getByTestId('debug-image'), 'busybox')
    expect(screen.getByTestId('debug-summary')).toHaveTextContent(
      'pod default/web-0 · target app · image busybox',
    )
    await user.click(screen.getByTestId('debug-submit'))
    expect(onConfirm).toHaveBeenCalledWith({ image: 'busybox', targetContainer: 'app' })
  })

  it('multi-container pods need an explicit target; Cancel sends nothing', async () => {
    const user = userEvent.setup()
    const { onConfirm, onCancel } = setup(['app', 'sidecar'])
    expect(screen.getByTestId('debug-target-container')).toHaveValue('')
    await user.type(screen.getByTestId('debug-image'), 'busybox')
    expect(screen.getByTestId('debug-submit')).toBeDisabled()
    await user.type(screen.getByTestId('debug-target-container'), 'sidecar')
    expect(screen.getByTestId('debug-submit')).toBeEnabled()
    await user.click(screen.getByTestId('debug-cancel'))
    expect(onCancel).toHaveBeenCalled()
    expect(onConfirm).not.toHaveBeenCalled()
  })
})

describe('addEphemeralContainerAction', () => {
  const a = addEphemeralContainerAction<{ namespace: string; name: string }>()
  it('plans one AddEphemeralContainer, nothing without an image', () => {
    expect(a.destructive).toBe(true)
    expect(a.plan([item], {})).toEqual([])
    expect(a.plan([item], { image: 'busybox', targetContainer: 'app' })[0]?.cmd).toEqual({
      type: 'AddEphemeralContainer',
      data: {
        namespace: 'default',
        pod: 'web-0',
        target_container: 'app',
        image: 'busybox',
        ephemeral_container_name: null,
      },
    })
  })
})
