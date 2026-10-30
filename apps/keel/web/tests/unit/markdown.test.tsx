import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import Markdown from '@/ui/Markdown'

function md(text: string, guideLinks = false): HTMLElement {
  const { container } = render(<Markdown text={text} guideLinks={guideLinks} />)
  const root = container.querySelector('[data-markdown]')
  if (!(root instanceof HTMLElement)) throw new Error('no markdown root')
  return root
}

describe('Markdown (spec 060 FR-012)', () => {
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
    expect(a).toHaveAttribute('target', '_blank')
    expect(a).toHaveAttribute('rel', 'noopener noreferrer')
  })
})

describe('Markdown guideLinks mode (spec 063 R6)', () => {
  it('gives h2 and h3 an id, numbering repeats', () => {
    const root = md("# Title\n\n## Enroll a host\n\n### What's a `token`?\n\n## Enroll a host", true)
    expect(root.querySelector('h1')?.id).toBe('')
    expect([...root.querySelectorAll('h2, h3')].map((h) => h.id)).toEqual([
      'enroll-a-host',
      'whats-a-token',
      'enroll-a-host-2',
    ])
  })

  it('leaves headings without ids in the default mode', () => {
    expect(md('## Enroll a host').querySelector('h2')?.id).toBe('')
  })

  it('turns a link to another guide into its help route, in the same tab', () => {
    md('[hosts](hosts.md#enroll-a-host) and [reviews](./reviews.md) and [index](README.md)', true)
    const hosts = screen.getByRole('link', { name: 'hosts' })
    expect(hosts).toHaveAttribute('href', '/help/hosts#enroll-a-host')
    expect(hosts).not.toHaveAttribute('target')
    expect(screen.getByRole('link', { name: 'reviews' })).toHaveAttribute('href', '/help/reviews')
    expect(screen.getByRole('link', { name: 'index' })).toHaveAttribute('href', '/help')
  })

  it('keeps an in-page anchor in the same tab', () => {
    md('[below](#deliveries)', true)
    const a = screen.getByRole('link', { name: 'below' })
    expect(a).toHaveAttribute('href', '#deliveries')
    expect(a).not.toHaveAttribute('target')
  })

  it('keeps the safety rules for everything else', () => {
    const root = md(
      '[out](https://example.com) [nope](nowhere.md) [js](javascript:alert(1)) <b>x</b>\n\n' +
        '![remote](https://example.com/d.png) ![missing](img/none.png)',
      true,
    )
    const out = screen.getByRole('link', { name: 'out' })
    expect(out).toHaveAttribute('target', '_blank')
    expect(out).toHaveAttribute('rel', 'noopener noreferrer')
    expect(root.querySelector('a[href="nowhere.md"]')).toBeNull()
    expect(root.textContent).toContain('nope')
    expect(root.querySelector('a[href^="javascript"]')).toBeNull()
    expect(root.querySelector('b')).toBeNull()
    expect(root.querySelector('img')).toBeNull()
    expect(screen.getByRole('link', { name: '[remote]' })).toHaveAttribute('data-image-link')
    expect(root.textContent).toContain('[missing]')
  })
})
