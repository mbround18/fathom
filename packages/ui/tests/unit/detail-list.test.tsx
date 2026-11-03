import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { DetailList, DetailRow, Unknown } from '../../src/kit'

describe('DetailList', () => {
  it('pairs each term with its value, and names the list when asked', () => {
    render(
      <DetailList label="Widget">
        <DetailRow term="Name" data="name">
          blue-widget
        </DetailRow>
        <DetailRow term="Owner">
          <Unknown />
        </DetailRow>
      </DetailList>,
    )
    expect(document.querySelector('dl')).toHaveAttribute('aria-label', 'Widget')
    expect(screen.getAllByRole('term').map((t) => t.textContent)).toEqual(['Name', 'Owner'])
    expect(screen.getAllByRole('definition').map((d) => d.textContent)).toEqual(['blue-widget', 'unknown'])
    expect(document.querySelector('[data-field="name"]')).toHaveTextContent('blue-widget')
  })

  it('puts two pairs side by side only from the sm width up', () => {
    const { container } = render(
      <DetailList columns={2}>
        <DetailRow term="A">1</DetailRow>
      </DetailList>,
    )
    const cls = container.querySelector('dl')?.className ?? ''
    expect(cls).toContain('grid-cols-[auto_minmax(0,1fr)]')
    expect(cls).toContain('sm:grid-cols-[auto_minmax(0,1fr)_auto_minmax(0,1fr)]')
  })
})
