import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { Tabs } from '../../src/kit'

const items = [
  { value: 'overview', label: 'Overview' },
  { value: 'activity', label: 'Activity' },
] as const

describe('Tabs', () => {
  it('labels the list, marks each tab and renders only the active panel', () => {
    render(
      <Tabs label="Widget sections" items={items} value="activity" onChange={vi.fn()}>
        <p>Activity panel</p>
      </Tabs>,
    )
    expect(screen.getByRole('tablist', { name: 'Widget sections' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Overview' })).toHaveAttribute('data-tab', 'overview')
    expect(screen.getByRole('tab', { name: 'Activity' })).toHaveAttribute('aria-selected', 'true')
    expect(screen.getAllByRole('tabpanel')).toHaveLength(1)
    expect(screen.getByRole('tabpanel')).toHaveTextContent('Activity panel')
  })

  it('hands the chosen item’s value to onChange', async () => {
    const onChange = vi.fn()
    render(
      <Tabs label="Widget sections" items={items} value="overview" onChange={onChange}>
        <p>Overview panel</p>
      </Tabs>,
    )
    await userEvent.click(screen.getByRole('tab', { name: 'Activity' }))
    expect(onChange).toHaveBeenCalledWith('activity')
  })
})
