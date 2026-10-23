import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { LogView } from '../src/LogView'

/*
 * jsdom has no layout, so the scroll surface is given one: a 400px viewport, 20px rows, and a
 * content height the test controls. `scrollTop` gets a real backing value (jsdom's is inert).
 */
const VIEWPORT = 400
const ROW = 20
let contentHeight = 0
const scrollTops = new WeakMap<Element, number>()

const lines = (n: number, from = 0) => Array.from({ length: n }, (_, i) => `line ${from + i}`)

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(VIEWPORT)
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(800)
  vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(ROW)
  vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(() => contentHeight)
  Object.defineProperty(Element.prototype, 'scrollTop', {
    configurable: true,
    get(this: Element) {
      return scrollTops.get(this) ?? 0
    },
    set(this: Element, v: number) {
      scrollTops.set(this, Math.max(0, Math.min(v, contentHeight - VIEWPORT)))
    },
  })
})

afterEach(() => {
  // Restore jsdom's own accessor.
  delete (Element.prototype as { scrollTop?: number }).scrollTop
})

function surface() {
  return screen.getByTestId('log')
}

describe('LogView', () => {
  it('renders only the visible window of 10,000 lines', () => {
    contentHeight = 10_000 * ROW
    const { container } = render(
      <LogView data-testid="log" lines={lines(10_000)} follow={false} wrap />,
    )
    const rendered = container.querySelectorAll('[data-index]').length
    expect(rendered).toBeGreaterThan(0)
    // A 400px window of 20px rows is 20 rows; overscan may add a few, never thousands.
    expect(rendered).toBeLessThan(100)
    expect(screen.getByText('line 0')).toBeInTheDocument()
    expect(screen.queryByText('line 9999')).not.toBeInTheDocument()
  })

  it('renders a short log whole, so every line is in the document', () => {
    contentHeight = 300 * ROW
    const { container } = render(
      <LogView data-testid="log" lines={lines(300)} follow={false} wrap />,
    )
    expect(container.querySelectorAll('[data-index]')).toHaveLength(300)
    expect(screen.getByText('line 299')).toBeInTheDocument()
  })

  it('keeps the scroll anchor at the newest line while following', () => {
    contentHeight = 10_000 * ROW
    const { rerender } = render(<LogView data-testid="log" lines={lines(10_000)} follow wrap />)
    expect(surface().scrollTop).toBe(contentHeight - VIEWPORT)
    fireEvent.scroll(surface())
    expect(screen.getByText('line 9999')).toBeInTheDocument()

    contentHeight = 10_050 * ROW
    rerender(<LogView data-testid="log" lines={lines(10_050)} follow wrap />)
    expect(surface().scrollTop).toBe(contentHeight - VIEWPORT)
    fireEvent.scroll(surface())
    expect(screen.getByText('line 10049')).toBeInTheDocument()
  })

  it('leaves the view where the operator put it when not following', () => {
    contentHeight = 10_000 * ROW
    const { rerender } = render(
      <LogView data-testid="log" lines={lines(10_000)} follow={false} wrap />,
    )
    surface().scrollTop = 2_000
    fireEvent.scroll(surface())
    contentHeight = 10_050 * ROW
    rerender(<LogView data-testid="log" lines={lines(10_050)} follow={false} wrap />)
    expect(surface().scrollTop).toBe(2_000)
  })

  it('reports follow off when scrolled away from the bottom, and on when back', () => {
    contentHeight = 10_000 * ROW
    const onFollowChange = vi.fn()
    render(
      <LogView
        data-testid="log"
        lines={lines(10_000)}
        follow
        wrap
        onFollowChange={onFollowChange}
      />,
    )
    surface().scrollTop = 0
    fireEvent.scroll(surface())
    expect(onFollowChange).toHaveBeenLastCalledWith(false)
    surface().scrollTop = contentHeight
    fireEvent.scroll(surface())
    expect(onFollowChange).toHaveBeenLastCalledWith(true)
  })

  it('asks for older lines when scrolled to the top', () => {
    contentHeight = 10_000 * ROW
    const onLoadOlder = vi.fn()
    render(
      <LogView data-testid="log" lines={lines(10_000)} follow wrap onLoadOlder={onLoadOlder} />,
    )
    surface().scrollTop = 0
    fireEvent.scroll(surface())
    expect(onLoadOlder).toHaveBeenCalledTimes(1)
  })

  it('shows "N older lines dropped" when the buffer dropped lines', () => {
    contentHeight = 10 * ROW
    const { rerender } = render(
      <LogView data-testid="log" lines={lines(10)} dropped={15_000} follow wrap />,
    )
    expect(screen.getByText('15000 older lines dropped')).toBeInTheDocument()
    rerender(<LogView data-testid="log" lines={lines(10)} dropped={1} follow wrap />)
    expect(screen.getByText('1 older line dropped')).toBeInTheDocument()
    rerender(<LogView data-testid="log" lines={lines(10)} dropped={0} follow wrap />)
    expect(screen.queryByText(/older lines? dropped/)).not.toBeInTheDocument()
  })

  it('says older lines were dropped when the count is unknown', () => {
    contentHeight = 10 * ROW
    render(<LogView data-testid="log" lines={lines(10)} dropped follow wrap />)
    expect(screen.getByText('Older lines dropped')).toBeInTheDocument()
  })

  it('lets the caller word the marker', () => {
    contentHeight = 10 * ROW
    render(
      <LogView
        data-testid="log"
        lines={lines(10)}
        dropped={3}
        droppedText={(n) => `… ${String(n)} earlier lines truncated`}
        follow
        wrap
      />,
    )
    expect(screen.getByText('… 3 earlier lines truncated')).toBeInTheDocument()
  })

  it('shows the placeholder instead of lines when given one', () => {
    contentHeight = 0
    render(
      <LogView data-testid="log" lines={[]} follow wrap placeholder={<p>No log output yet.</p>} />,
    )
    expect(screen.getByText('No log output yet.')).toBeInTheDocument()
  })
})
