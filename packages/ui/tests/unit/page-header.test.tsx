import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { KitProvider, Page, PageHeader } from '../../src/kit'

const guides = { guideHref: (r: { guide: string }) => `/help/${r.guide}` }

describe('PageHeader', () => {
  it('labels its page by the h1', () => {
    render(
      <Page name="widgets" labelledBy="widgets-title">
        <PageHeader id="widgets-title" title="Widgets" />
      </Page>,
    )
    expect(screen.getByRole('region', { name: 'Widgets' })).toHaveAttribute('data-page', 'widgets')
    expect(screen.getByRole('heading', { level: 1, name: 'Widgets' })).toHaveAttribute('id', 'widgets-title')
  })

  it('shows a plain description without a guide', () => {
    const { container } = render(
      <KitProvider value={guides}>
        <PageHeader id="t" title="Widgets" description="Everything listed." />
      </KitProvider>,
    )
    expect(screen.getByText('Everything listed.')).toBeInTheDocument()
    expect(container.querySelector('[data-page-intro]')).toBeNull()
    expect(screen.queryByRole('link')).toBeNull()
  })

  it('follows the description with Learn more when it has a guide', () => {
    const { container } = render(
      <KitProvider value={guides}>
        <PageHeader id="t" title="Widgets" description="Everything listed." guide={{ guide: 'widgets' }} />
      </KitProvider>,
    )
    expect(container.querySelector('[data-page-intro]')).not.toBeNull()
    expect(screen.getByRole('link', { name: 'Learn more' })).toHaveAttribute('href', '/help/widgets')
  })

  it('keeps the tip outside the heading, so the heading’s name stays the title', () => {
    render(<PageHeader id="t" title="Widgets" tip={<button aria-label="About widgets" />} />)
    expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Widgets')
    expect(screen.getByRole('button', { name: 'About widgets' })).toBeInTheDocument()
  })
})
