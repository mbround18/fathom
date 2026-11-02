/** Ported from Keel's tests/unit/markdown.test.tsx, over the kit's KitProvider instead of Keel's. */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { KitProvider, Markdown, type KitContext } from '../../src/kit'

const KIT: KitContext = {
  guideHref: (r) => `/help/${r.guide}${r.anchor ? `#${r.anchor}` : ''}`,
  isGuide: (s) => s === 'widgets' || s === 'gadgets',
  guideImage: (name) => (name === 'share-menu.png' ? '/assets/share-menu.png' : undefined),
}

function md(text: string, guideLinks = false, kit: KitContext = KIT): HTMLElement {
  const { container } = render(
    <KitProvider value={kit}>
      <Markdown text={text} guideLinks={guideLinks} />
    </KitProvider>,
  )
  const root = container.querySelector('[data-markdown]')
  if (!(root instanceof HTMLElement)) throw new Error('no markdown root')
  return root
}

describe('Markdown', () => {
  it('renders GFM', () => {
    const root = md('# Title\n\n- one\n- [x] two\n\n| a | b |\n|---|---|\n| 1 | 2 |')
    expect(root.querySelector('h1')?.textContent).toBe('Title')
    expect(root.querySelectorAll('li')).toHaveLength(2)
    expect(root.querySelector('table')).not.toBeNull()
  })

  it('shows raw HTML as text, never as elements', () => {
    const root = md('Hello <script>alert(1)</script> and <b>bold</b>\n\n<div onclick="x()">block</div>')
    expect(root.querySelector('script')).toBeNull()
    expect(root.querySelector('b')).toBeNull()
    expect(root.querySelector('div[onclick]')).toBeNull()
    expect(root.textContent).toContain('<script>alert(1)</script>')
    expect(root.textContent).toContain('<div onclick="x()">block</div>')
  })

  it('opens links in a new tab without an opener', () => {
    md('[docs](https://example.com/docs)')
    const a = screen.getByRole('link', { name: 'docs' })
    expect(a).toHaveAttribute('href', 'https://example.com/docs')
    expect(a).toHaveAttribute('target', '_blank')
    expect(a).toHaveAttribute('rel', 'noopener noreferrer')
  })

  it('never follows a javascript: link', () => {
    const root = md('[click](javascript:alert(1))')
    expect(root.querySelector('a')).toBeNull()
    expect(root.textContent).toContain('click')
  })

  it('turns images into links and never fetches them', () => {
    const root = md('![the diagram](https://example.com/d.png)')
    expect(root.querySelector('img')).toBeNull()
    const a = screen.getByRole('link', { name: '[the diagram]' })
    expect(a).toHaveAttribute('href', 'https://example.com/d.png')
    expect(a).toHaveAttribute('data-image-link')
    expect(a).toHaveAttribute('rel', 'noopener noreferrer')
  })

  it('leaves guide links and img/ images alone outside guide mode', () => {
    const root = md('[widgets](widgets.md) ![menu](img/share-menu.png)')
    expect(root.querySelector('a')).toBeNull()
    expect(root.querySelector('img')).toBeNull()
  })
})

describe('Markdown guideLinks mode', () => {
  it('gives h2 and h3 an id, numbering repeats', () => {
    const root = md("# Title\n\n## Share a widget\n\n### What's a `token`?\n\n## Share a widget", true)
    expect(root.querySelector('h1')?.id).toBe('')
    expect([...root.querySelectorAll('h2, h3')].map((h) => h.id)).toEqual(['share-a-widget', 'whats-a-token', 'share-a-widget-2'])
  })

  it('leaves headings without ids in the default mode', () => {
    expect(md('## Share a widget').querySelector('h2')?.id).toBe('')
  })

  it('turns a link to another guide into its route, in the same tab', () => {
    md('[widgets](widgets.md#sharing) and [gadgets](./gadgets.md)', true)
    const widgets = screen.getByRole('link', { name: 'widgets' })
    expect(widgets).toHaveAttribute('href', '/help/widgets#sharing')
    expect(widgets).not.toHaveAttribute('target')
    expect(screen.getByRole('link', { name: 'gadgets' })).toHaveAttribute('href', '/help/gadgets')
  })

  it('keeps an in-page anchor in the same tab', () => {
    md('[below](#removing)', true)
    const a = screen.getByRole('link', { name: 'below' })
    expect(a).toHaveAttribute('href', '#removing')
    expect(a).not.toHaveAttribute('target')
  })

  it('shows an img/ image the app bundles, and keeps any other image a link', () => {
    const root = md('![the share menu](img/share-menu.png) ![missing](img/none.png) ![remote](https://example.com/d.png)', true)
    const img = screen.getByRole('img', { name: 'the share menu' })
    expect(img).toHaveAttribute('src', '/assets/share-menu.png')
    expect(root.querySelectorAll('img')).toHaveLength(1)
    expect(root.textContent).toContain('[missing]')
    expect(screen.getByRole('link', { name: '[remote]' })).toHaveAttribute('data-image-link')
  })

  it('keeps the safety rules for everything else', () => {
    const root = md('[out](https://example.com) [nope](nowhere.md) [js](javascript:alert(1)) <b>x</b>', true)
    expect(screen.getByRole('link', { name: 'out' })).toHaveAttribute('target', '_blank')
    expect(root.querySelector('a[href="nowhere.md"]')).toBeNull()
    expect(root.textContent).toContain('nope')
    expect(root.querySelector('a[href^="javascript"]')).toBeNull()
    expect(root.querySelector('b')).toBeNull()
  })

  it('without the app’s guide hooks, a guide link is not a link', () => {
    const root = md('[widgets](widgets.md)', true, {})
    expect(root.querySelector('a')).toBeNull()
    expect(root.textContent).toContain('widgets')
  })
})
