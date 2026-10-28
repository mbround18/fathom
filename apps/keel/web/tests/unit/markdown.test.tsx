import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import Markdown from '@/features/settings/Markdown'

function md(text: string): HTMLElement {
  const { container } = render(<Markdown text={text} />)
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
