/** Picker (T064): an open picker stays open when its options grow past the search threshold. */
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeAll, describe, expect, it, vi } from 'vitest'

import { Picker, SEARCH_THRESHOLD, type PickerOption } from '@/components/Picker'

const opts = (n: number): PickerOption[] =>
  Array.from({ length: n }, (_, i) => ({ value: `ns-${String(i)}`, label: `ns-${String(i)}` }))

describe('Picker', () => {
  beforeAll(() => {
    // jsdom has no layout APIs; Radix Select and cmdk use both.
    Element.prototype.scrollIntoView = vi.fn()
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    )
  })

  it('stays open, now searchable, when options arrive while it is open', async () => {
    const user = userEvent.setup()
    const props = { label: 'Namespace', prefix: 'ns', value: 'ns-0', onChange: vi.fn() }
    const { rerender } = render(<Picker {...props} options={opts(1)} />)

    screen.getByRole('combobox', { name: 'Namespace' }).focus()
    await user.keyboard('{Enter}')
    expect(await screen.findByRole('option', { name: 'ns-0' })).toBeTruthy()

    rerender(<Picker {...props} options={opts(SEARCH_THRESHOLD + 2)} />)
    const late = `ns-${String(SEARCH_THRESHOLD + 1)}`
    expect(await screen.findByRole('option', { name: late })).toBeTruthy()

    await user.click(screen.getByRole('option', { name: late }))
    expect(props.onChange).toHaveBeenCalledWith(late)
  })
})
