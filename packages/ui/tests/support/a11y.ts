/**
 * The accessibility check every piece spec runs on every state (spec 065 FR-017, SC-007): axe with
 * the WCAG 2.2 AA tags, in the dark and the light theme, at a phone and a desktop width, with zero
 * violations allowed, and the page never scrolling sideways (constitution IV).
 *
 * `KIT_SHOTS=<dir>` also saves each state, theme and width, for the impeccable review (FR-018).
 */
import AxeBuilder from '@axe-core/playwright'
import { expect, type Page } from '@playwright/test'

const THEMES = ['dark', 'light'] as const
const SIZES = [
  { width: 360, height: 740 },
  { width: 1280, height: 800 },
] as const
const TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa']

export interface At {
  piece: string
  state: string
}

/** The harness URL for a piece's state. */
export function harnessUrl(at: At, theme: 'dark' | 'light' = 'dark'): string {
  return `/?${new URLSearchParams({ piece: at.piece, state: at.state, theme }).toString()}`
}

/** Opens a piece's state in the harness and waits for it to settle. */
export async function openPiece(page: Page, at: At, theme: 'dark' | 'light' = 'dark'): Promise<void> {
  await page.goto(harnessUrl(at, theme))
  await page.waitForFunction(() => window.__kitReady === true)
}

/**
 * Scans the state in each theme and width. `prepare` brings the page to the state to scan (opens a
 * dialog, a popover) after each load; `disable` turns off named rules for this piece only, each
 * with its reason in the calling spec.
 */
export async function expectAccessible(
  page: Page,
  at: At,
  opts: { prepare?: (page: Page) => Promise<void>; disable?: readonly string[] } = {},
): Promise<void> {
  for (const theme of THEMES) {
    for (const size of SIZES) {
      const where = `${at.piece}/${at.state} ${theme} ${String(size.width)}px`
      await page.setViewportSize(size)
      await openPiece(page, at, theme)
      await opts.prepare?.(page)
      if (process.env.KIT_SHOTS) {
        await page.screenshot({ path: `${process.env.KIT_SHOTS}/${at.piece}-${at.state}-${theme}-${String(size.width)}.png`, fullPage: true })
      }
      const result = await new AxeBuilder({ page })
        .withTags(TAGS)
        .disableRules([...(opts.disable ?? [])])
        .analyze()
      const found = result.violations.map((v) => `${v.id} (${v.impact ?? 'unknown'}): ${v.nodes[0]?.target.join(' ') ?? ''}`)
      expect(found, `${where}: axe violations`).toEqual([])
      const [scroll, client] = await page.evaluate(() => [document.documentElement.scrollWidth, document.documentElement.clientWidth])
      expect(scroll, `${where}: the page scrolls sideways`).toBeLessThanOrEqual(client)
    }
  }
}

declare global {
  interface Window {
    __kitReady?: boolean
  }
}
