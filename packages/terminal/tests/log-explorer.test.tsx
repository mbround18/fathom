import { act, fireEvent, render, screen, within } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { LogExplorer } from '../src/LogExplorer'
import { FLASH_MS } from '../src/LogView'

const E = '\u001b'
const VIEWPORT = 400
const ROW = 20
let contentHeight = 0
const scrollTops = new WeakMap<Element, number>()

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
  delete (Element.prototype as { scrollTop?: number }).scrollTop
  vi.useRealTimers()
})

const LINES = [
  `2026-10-23T12:00:01Z ${E}[32mINFO${E}[0m booting`,
  `2026-10-23T12:00:02Z ${E}[1;31mERROR${E}[0m db down ${E}]8;;https://evil${E}\\link${E}]8;;${E}\\`,
  '{"level":"warn","msg":"slow","service":"api"}',
  '<img src=x onerror="window.__pwned=1"> GET /health 200',
]

function Harness({ lines = LINES, onFollow }: { lines?: string[]; onFollow?: (f: boolean) => void }) {
  const [follow, setFollow] = useState(true)
  return (
    <LogExplorer
      data-testid="log"
      lines={lines}
      follow={follow}
      wrap
      onFollowChange={(f) => {
        setFollow(f)
        onFollow?.(f)
      }}
      toolbarStart={<span data-follow={String(follow)} />}
    />
  )
}

const log = () => screen.getByTestId('log')
const search = () => screen.getByRole('searchbox', { name: 'Search logs' })
const row = (line: number) => log().querySelector<HTMLElement>(`[data-line="${String(line)}"]`)

describe('LogExplorer: rendering', () => {
  it('renders SGR colour as styled spans and removes every other sequence', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    const error = row(1)
    expect(error?.textContent).toBe('2026-10-23T12:00:02Z ERROR db down link\n')
    expect(log().textContent).not.toContain(E)
    const span = error?.querySelector('[data-ansi]')
    expect(span?.textContent).toBe('ERROR')
    expect(span).toHaveAttribute('data-fg', '1')
    expect(span).toHaveAttribute('data-bold')
    expect(row(0)?.querySelector('[data-ansi]')).toHaveAttribute('data-fg', '2')
    // Markup in a line is text.
    expect(log().querySelector('img')).toBeNull()
    expect(row(3)?.textContent).toContain('<img src=x onerror="window.__pwned=1">')
  })

  it('marks each line with its detected level', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    expect(row(0)).toHaveAttribute('data-level', 'info')
    expect(row(1)).toHaveAttribute('data-level', 'error')
    expect(row(2)).toHaveAttribute('data-level', 'warn')
    expect(row(3)).not.toHaveAttribute('data-level')
  })

  it('offers level chips with counts', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    expect(screen.getByRole('button', { name: 'Level error, 1 lines' })).toHaveAttribute('aria-pressed', 'false')
    expect(screen.getByRole('button', { name: 'Level warn, 1 lines' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Level debug/ })).not.toBeInTheDocument()
  })
})

describe('LogExplorer: search', () => {
  it('narrows, counts and highlights the matched text', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    fireEvent.change(search(), { target: { value: 'db DOWN' } })
    expect(log()).toHaveAttribute('data-view', 'results')
    expect(screen.getByText('1 of 4', { selector: '[data-field="matches"]' })).toBeInTheDocument()
    const marks = log().querySelectorAll('mark')
    expect([...marks].map((m) => m.textContent)).toEqual(['db', 'down'])
    expect(row(0)).toBeNull()
  })

  it('highlights across colour runs', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    fireEvent.change(search(), { target: { value: '"error db"' } })
    const marks = [...log().querySelectorAll('mark')].map((m) => m.textContent)
    expect(marks.join('')).toBe('ERROR db')
  })

  it('filters by level and by field', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    fireEvent.change(search(), { target: { value: 'level:warn service:api' } })
    expect(row(2)).not.toBeNull()
    expect(log().querySelectorAll('[data-line]')).toHaveLength(1)
    fireEvent.change(search(), { target: { value: '-level:info -health' } })
    expect([...log().querySelectorAll('[data-line]')].map((r) => r.getAttribute('data-line'))).toEqual(['1', '2'])
  })

  it('says when nothing matches, and Escape clears the search', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    fireEvent.change(search(), { target: { value: 'nothing-matches' } })
    expect(screen.getByText('No line matches.')).toBeInTheDocument()
    fireEvent.keyDown(search(), { key: 'Escape' })
    expect(search()).toHaveValue('')
    expect(log()).toHaveAttribute('data-view', 'all')
  })

  it('level chips toggle a level filter and keep their counts', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    fireEvent.click(screen.getByRole('button', { name: 'Level error, 1 lines' }))
    expect(screen.getByRole('button', { name: 'Level error, 1 lines' })).toHaveAttribute('aria-pressed', 'true')
    expect([...log().querySelectorAll('[data-line]')].map((r) => r.getAttribute('data-line'))).toEqual(['1'])
    // The other chips still count what the query leaves.
    expect(screen.getByRole('button', { name: 'Level warn, 1 lines' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Level error, 1 lines' }))
    expect(log()).toHaveAttribute('data-view', 'all')
  })
})

describe('LogExplorer: show in context', () => {
  it('clicking a result shows the whole log at that line, flashes it, and pauses follow', () => {
    vi.useFakeTimers()
    contentHeight = 4 * ROW
    const onFollow = vi.fn()
    render(<Harness onFollow={onFollow} />)
    fireEvent.change(search(), { target: { value: 'slow' } })
    const result = within(row(2) as HTMLElement).getByRole('button', { name: /slow/ })
    fireEvent.click(result)

    expect(log()).toHaveAttribute('data-view', 'context')
    expect(log().querySelectorAll('[data-line]')).toHaveLength(4)
    expect(row(2)).toHaveAttribute('data-flash')
    expect(document.activeElement).toBe(row(2))
    expect(onFollow).toHaveBeenLastCalledWith(false)
    // The query stays, and still highlights, but rows are no longer buttons.
    expect(search()).toHaveValue('slow')
    expect(log().querySelector('mark')?.textContent).toBe('slow')
    expect(log().querySelector('[data-action="show-in-context"]')).toBeNull()

    act(() => {
      vi.advanceTimersByTime(FLASH_MS + 10)
    })
    expect(row(2)).not.toHaveAttribute('data-flash')

    fireEvent.click(screen.getByRole('button', { name: /Back to results/ }))
    expect(log()).toHaveAttribute('data-view', 'results')
    expect(log().querySelectorAll('[data-line]')).toHaveLength(1)
  })

  it('Enter and Space on a result activate it', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    fireEvent.change(search(), { target: { value: 'booting' } })
    const result = within(row(0) as HTMLElement).getByRole('button', { name: /booting/ })
    expect(result).toHaveAttribute('tabindex', '0')
    fireEvent.keyDown(result, { key: 'Enter' })
    expect(log()).toHaveAttribute('data-view', 'context')
    fireEvent.click(screen.getByRole('button', { name: /Back to results/ }))
    fireEvent.keyDown(within(row(0) as HTMLElement).getByRole('button', { name: /booting/ }), { key: ' ' })
    expect(log()).toHaveAttribute('data-view', 'context')
  })

  it('typing again leaves context for the new results', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    fireEvent.change(search(), { target: { value: 'booting' } })
    fireEvent.click(within(row(0) as HTMLElement).getByRole('button', { name: /booting/ }))
    fireEvent.change(search(), { target: { value: 'db' } })
    expect(log()).toHaveAttribute('data-view', 'results')
  })

  it('jumps to a line deep in a virtualised log', () => {
    const many = Array.from({ length: 5_000 }, (_, i) => (i === 4_321 ? 'needle in the haystack' : `line ${String(i)}`))
    contentHeight = many.length * ROW
    render(<Harness lines={many} />)
    fireEvent.change(search(), { target: { value: 'needle' } })
    expect(log().querySelectorAll('[data-line]')).toHaveLength(1)
    fireEvent.click(within(row(4_321) as HTMLElement).getByRole('button', { name: /needle/ }))
    expect(log()).toHaveAttribute('data-view', 'context')
    // The virtualiser scrolled the jumped-to line into the window.
    fireEvent.scroll(log())
    expect(log().scrollTop).toBeGreaterThan(4_000 * ROW)
    expect(row(4_321)).not.toBeNull()
    expect(row(4_321)).toHaveAttribute('data-flash')
    expect(log().querySelectorAll('[data-line]').length).toBeLessThan(100)
  })
})

describe('LogExplorer: JSON', () => {
  it('expands a JSON line to a pretty view, separately from show-in-context', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    fireEvent.change(search(), { target: { value: 'slow' } })
    const toggle = screen.getByRole('button', { name: 'Expand JSON' })
    fireEvent.click(toggle)
    // Still the results view: expanding is not a jump.
    expect(log()).toHaveAttribute('data-view', 'results')
    const pretty = row(2)?.querySelector('[data-json-pretty]')
    expect(pretty?.textContent).toBe('{\n  "level": "warn",\n  "msg": "slow",\n  "service": "api"\n}')
    expect(screen.getByRole('button', { name: 'Collapse JSON' })).toHaveAttribute('aria-expanded', 'true')
    fireEvent.click(screen.getByRole('button', { name: 'Collapse JSON' }))
    expect(row(2)?.querySelector('[data-json-pretty]')).toBeNull()
  })

  it('offers no JSON toggle on text lines', () => {
    contentHeight = 4 * ROW
    render(<Harness />)
    expect(screen.getAllByRole('button', { name: 'Expand JSON' })).toHaveLength(1)
  })
})
