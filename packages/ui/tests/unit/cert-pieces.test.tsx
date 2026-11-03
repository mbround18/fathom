import { fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { ChipList, PemField, StageList } from '../../src/kit'
import { CERT, KEY } from './fixtures/pem'

describe('ChipList', () => {
  const names = ['a.example.test', 'b.example.test', 'c.example.test', 'd.example.test']

  it('shows up to max, folds the rest into +N more, and folds back', () => {
    render(<ChipList label="Names" items={names} max={2} />)
    expect(screen.getAllByRole('listitem').map((li) => li.textContent)).toEqual(names.slice(0, 2))
    const more = screen.getByRole('button', { name: '+2 more' })
    expect(more).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(more)
    expect(screen.getAllByRole('listitem')).toHaveLength(4)
    fireEvent.click(screen.getByRole('button', { name: 'Show fewer' }))
    expect(screen.getAllByRole('listitem')).toHaveLength(2)
  })

  it('has no button when everything fits, and says None when empty', () => {
    const { rerender } = render(<ChipList label="Names" items={names} />)
    expect(screen.queryByRole('button')).toBeNull()
    rerender(<ChipList label="Names" items={[]} />)
    expect(screen.getByText('None')).toBeInTheDocument()
  })
})

function Pem({ expect: types }: { expect?: readonly string[] }) {
  const [value, setValue] = useState('')
  return <PemField id="pem" label="Certificate" value={value} onChange={setValue} hint="The leaf first." {...(types ? { expect: types } : {})} />
}

describe('PemField', () => {
  it('shows the hint until something is pasted, then what it found', () => {
    render(<Pem expect={['CERTIFICATE']} />)
    const field = screen.getByLabelText('Certificate')
    expect(field).toHaveAccessibleDescription('The leaf first.')
    fireEvent.change(field, { target: { value: CERT } })
    expect(field).toHaveAccessibleDescription('1 block: CERTIFICATE')
    expect(field).toHaveAttribute('aria-invalid', 'false')
  })

  it('catches a private key pasted where a certificate goes', () => {
    render(<Pem expect={['CERTIFICATE']} />)
    const field = screen.getByLabelText('Certificate')
    fireEvent.change(field, { target: { value: KEY } })
    expect(field).toHaveAttribute('aria-invalid', 'true')
    expect(field).toHaveAccessibleDescription('Expected CERTIFICATE, found PRIVATE KEY.')
  })

  it('loads a file into the field', async () => {
    render(<Pem />)
    const input = document.querySelector<HTMLInputElement>('input[type="file"]')
    if (!input) throw new Error('no file input')
    const file = new File([CERT], 'leaf.pem')
    fireEvent.change(input, { target: { files: [file] } })
    await vi.waitFor(() => {
      expect(screen.getByLabelText('Certificate')).toHaveValue(CERT)
    })
  })
})

describe('StageList', () => {
  it('lists the stages in order, marks the current one and says which is stuck', () => {
    const open = vi.fn()
    render(
      <StageList
        label="Issuance"
        stages={[
          { key: 'cert', title: 'Certificate widgets', state: 'done', status: 'Ready' },
          { key: 'order', title: 'Order widgets-1', state: 'current' },
          { key: 'challenge', title: 'Challenge widgets-1-0', state: 'stuck', note: 'DNS record not found', onOpen: open },
          { key: 'next', title: 'Renewal', state: 'pending' },
        ]}
      />,
    )
    const items = screen.getAllByRole('listitem')
    expect(items.map((li) => li.getAttribute('data-state'))).toEqual(['done', 'current', 'stuck', 'pending'])
    expect(items[1]).toHaveAttribute('aria-current', 'step')
    expect(items[2]).toHaveTextContent('Stuck')
    expect(items[2]).toHaveTextContent('DNS record not found')
    fireEvent.click(screen.getByRole('button', { name: /Challenge widgets-1-0/ }))
    expect(open).toHaveBeenCalledOnce()
    expect(screen.getAllByRole('button')).toHaveLength(1)
  })
})
