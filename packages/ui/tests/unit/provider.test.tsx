import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { KitProvider, useKit } from '../../src/kit'

function Probe() {
  const kit = useKit()
  return <p>{kit.guideHref?.({ guide: 'widgets', anchor: 'add' }) ?? 'none'}</p>
}

describe('KitProvider', () => {
  it('hands its value to the pieces below it', () => {
    render(
      <KitProvider value={{ guideHref: (r) => `/help/${r.guide}#${r.anchor ?? ''}` }}>
        <Probe />
      </KitProvider>,
    )
    expect(screen.getByText('/help/widgets#add')).toBeInTheDocument()
  })

  it('is empty without a provider, so pieces render without app content', () => {
    render(<Probe />)
    expect(screen.getByText('none')).toBeInTheDocument()
  })
})
