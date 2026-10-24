import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { CodeEditor } from '@/features/editor/CodeEditor'

const content = (root: HTMLElement) => root.querySelector('.cm-content')

describe('CodeEditor', () => {
  it('is editable by default, as the compose editor uses it', () => {
    const { container } = render(<CodeEditor value="a: 1\n" onChange={() => undefined} language="yaml" label="compose.yaml" problems={[]} />)
    expect(content(container)?.getAttribute('contenteditable')).toBe('true')
    expect(content(container)?.hasAttribute('aria-readonly')).toBe(false)
    expect(container.querySelector('[data-code-editor="yaml"]')?.hasAttribute('data-read-only')).toBe(false)
  })

  it('readOnly marks the view read-only and can be lifted without a new view', () => {
    const { container, rerender } = render(<CodeEditor value='{"a": 1}' language="json" label="ops.json" readOnly />)
    const before = content(container)
    expect(before?.getAttribute('aria-readonly')).toBe('true')
    expect(container.querySelector('[data-code-editor="json"]')?.getAttribute('data-read-only')).toBe('true')
    rerender(<CodeEditor value='{"a": 1}' language="json" label="ops.json" />)
    expect(content(container)).toBe(before)
    expect(content(container)?.hasAttribute('aria-readonly')).toBe(false)
  })
})
