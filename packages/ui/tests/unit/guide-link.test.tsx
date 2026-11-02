import { render, screen } from '@testing-library/react'
import type { ComponentProps } from 'react'
import { describe, expect, it } from 'vitest'

import { GuideLink, KitProvider } from '../../src/kit'

const guideHref = (r: { guide: string; anchor?: string }) => `/help/${r.guide}${r.anchor ? `#${r.anchor}` : ''}`

describe('GuideLink', () => {
  it('links to the route the app builds, marked with the guide', () => {
    render(
      <KitProvider value={{ guideHref }}>
        <GuideLink to={{ guide: 'widgets', anchor: 'sharing' }} />
      </KitProvider>,
    )
    const link = screen.getByRole('link', { name: 'Learn more' })
    expect(link).toHaveAttribute('href', '/help/widgets#sharing')
    expect(link).toHaveAttribute('data-guide-link', 'widgets')
    expect(link).not.toHaveAttribute('target')
  })

  it('takes the caller’s words', () => {
    render(
      <KitProvider value={{ guideHref }}>
        <GuideLink to={{ guide: 'widgets' }}>Read the widgets guide</GuideLink>
      </KitProvider>,
    )
    expect(screen.getByRole('link', { name: 'Read the widgets guide' })).toHaveAttribute('href', '/help/widgets')
  })

  it('renders nothing when the app has no guides', () => {
    const { container } = render(<GuideLink to={{ guide: 'widgets' }} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('uses the app’s Link when it hands one over', () => {
    const AppLink = ({ to, children, ...rest }: Omit<ComponentProps<'a'>, 'href'> & { to: string }) => (
      <a href={`#app${to}`} data-app-link {...rest}>
        {children}
      </a>
    )
    render(
      <KitProvider value={{ guideHref, Link: AppLink }}>
        <GuideLink to={{ guide: 'widgets' }} />
      </KitProvider>,
    )
    const link = screen.getByRole('link', { name: 'Learn more' })
    expect(link).toHaveAttribute('data-app-link')
    expect(link).toHaveAttribute('href', '#app/help/widgets')
  })
})
