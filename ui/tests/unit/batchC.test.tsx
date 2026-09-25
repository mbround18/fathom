import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { descriptorFor } from '@/resources'
import { parseQuantity } from '@/resources/batchCCommon'
import { pdbStatus } from '@/resources/configKinds'
import { SecretKeys } from '@/resources/secretKeys'

const SECRET_VALUE = 'hunter2-super-secret'

describe('SecretKeys', () => {
  it('renders key names but no values until reveal, and re-masks', () => {
    const { container } = render(
      <SecretKeys keys={['password', 'user']} values={{ password: SECRET_VALUE, user: 'admin' }} />,
    )
    expect(screen.getByText('password')).toBeTruthy()
    expect(container.innerHTML).not.toContain(SECRET_VALUE)
    expect(container.innerHTML).not.toContain('admin')
    expect(screen.queryByLabelText('Copy value of password')).toBeNull()

    fireEvent.click(screen.getByLabelText('Reveal value of password'))
    expect(container.innerHTML).toContain(SECRET_VALUE)
    expect(container.innerHTML).not.toContain('admin')
    expect(screen.getByLabelText('Copy value of password')).toBeTruthy()

    fireEvent.keyDown(screen.getByTestId('secret-keys'), { key: 'Escape' })
    expect(container.innerHTML).not.toContain(SECRET_VALUE)
  })

  it('re-masks on blur out of the section and on unmount', () => {
    const { container, unmount } = render(
      <SecretKeys keys={['password']} values={{ password: SECRET_VALUE }} />,
    )
    fireEvent.click(screen.getByLabelText('Reveal value of password'))
    expect(container.innerHTML).toContain(SECRET_VALUE)
    fireEvent.blur(screen.getByTestId('secret-keys'), { relatedTarget: document.body })
    expect(container.innerHTML).not.toContain(SECRET_VALUE)
    unmount()
    expect(document.body.innerHTML).not.toContain(SECRET_VALUE)
  })

  it('says values are not transferred when the backend supplies none', () => {
    render(<SecretKeys keys={['token']} />)
    fireEvent.click(screen.getByLabelText('Reveal value of token'))
    expect(screen.getByTestId('secret-value-token').textContent).toContain('not transferred')
    expect(screen.queryByLabelText('Copy value of token')).toBeNull()
  })
})

describe('batch C descriptors', () => {
  it('registers all eight sections with delete gated and confirm-prompted', () => {
    for (const s of [
      'storage/persistentvolumes',
      'storage/persistentvolumeclaims',
      'storage/storageclasses',
      'config/configmaps',
      'config/secrets',
      'config/resourcequotas',
      'config/limitranges',
      'config/poddisruptionbudgets',
    ]) {
      const d = descriptorFor(s)
      expect(d, s).toBeDefined()
      const del = d?.actions.find((a) => a.id === 'delete')
      expect(del?.prompt).toBe('confirm')
      expect(del && 'accessCheck' in del).toBe(true)
    }
  })

  it('cluster-scoped deletes send name only', () => {
    const d = descriptorFor('storage/storageclasses')
    const row = {
      name: 'standard',
      provisioner: 'p',
      reclaim_policy: null,
      volume_binding_mode: null,
      is_default: false,
    }
    expect(d?.actions[0]?.plan([row as never], {})[0]?.cmd).toEqual({
      type: 'DeleteStorageClass',
      data: { name: 'standard' },
    })
  })

  it('secret detail keys section shows names only', () => {
    const d = descriptorFor('config/secrets')
    const detail = {
      namespace: 'default',
      name: 's',
      creation_timestamp: '2024-01-01T00:00:00Z',
      labels: {},
      annotations: {},
      secret_type: 'Opaque',
      keys: ['password'],
      raw_json: '{}',
    }
    const block = d?.detailSections.find((s) => s.id === 'keys')?.render({ detail })
    expect(block?.type).toBe('list')
    const { container } = render(<>{block?.type === 'list' ? block.items : null}</>)
    expect(container.innerHTML).toContain('password')
    expect(screen.getByTestId('secret-value-password').textContent).toBe('••••••••')
  })

  it('helpers', () => {
    expect(parseQuantity('500m')).toBe(0.5)
    expect(parseQuantity('1Gi')).toBe(1024 ** 3)
    expect(pdbStatus({ current_healthy: 1, desired_healthy: 2 })).toBe('Disrupted')
    expect(pdbStatus({ current_healthy: null, desired_healthy: 2 })).toBe('Unknown')
  })
})
