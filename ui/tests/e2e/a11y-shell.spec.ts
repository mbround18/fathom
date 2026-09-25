/**
 * 032 (C): shell accessibility contracts, asserted directly (no axe dependency).
 */
import { expect, test, type Page } from '@playwright/test'

import { open } from './helpers'

const cls = (page: Page) => page.locator('html').getAttribute('data-width-class')

test.describe('shell accessibility', () => {
  test('one h1, a working skip link, and a main landmark', async ({ page }) => {
    await open(page)
    await expect(page.locator('h1')).toHaveCount(1)
    await page.keyboard.press('Tab')
    const skip = page.getByRole('link', { name: 'Skip to main content' })
    await expect(skip).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(page.locator('main#main-content')).toBeFocused()
  })

  test('decorative kanji glyphs are hidden from assistive tech', async ({ page }) => {
    await open(page)
    const exposed = await page.evaluate(
      () =>
        [...document.querySelectorAll('.font-brand')].filter(
          (el) => !el.closest('[aria-hidden="true"]'),
        ).length,
    )
    expect(exposed).toBe(0)
  })

  test('active navigation entry carries aria-current', async ({ page }) => {
    await open(page)
    if ((await cls(page)) === 'phone' || (await cls(page)) === 'tablet') {
      await page.getByRole('button', { name: 'Open sections' }).click()
    }
    await expect(page.getByRole('navigation', { name: 'Sections' })).toBeVisible()
    await expect(
      page.getByRole('navigation', { name: 'Sections' }).locator('[aria-current="page"]'),
    ).toHaveCount(1)
  })

  test('resize handles are named, focusable and keyboard operable', async ({ page }) => {
    await open(page)
    test.skip(['phone', 'tablet'].includes((await cls(page)) ?? ''), 'no resize handles')
    const handles = page.getByRole('separator')
    const n = await handles.count()
    expect(n).toBeGreaterThan(0)
    for (let i = 0; i < n; i++) {
      const h = handles.nth(i)
      await expect(h).toHaveAttribute('aria-label', /Resize/)
      await expect(h).toHaveAttribute('tabindex', '0')
    }
    const first = handles.first()
    const before = await first.getAttribute('aria-valuenow')
    await first.focus()
    await page.keyboard.press('ArrowRight')
    await page.keyboard.press('ArrowDown')
    const after = await first.getAttribute('aria-valuenow')
    expect(after).not.toBe(before)
  })

  test('palette focuses its input and returns focus on close', async ({ page }) => {
    await open(page)
    const trigger = page.getByTestId('palette-open')
    await trigger.focus()
    await page.keyboard.press('Enter')
    await expect(page.getByRole('combobox').last()).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(trigger).toBeFocused()
  })

  test('dialog closing returns focus to the trigger', async ({ page }) => {
    await open(page)
    test.skip((await cls(page)) === 'phone', 'fleet lives in the overflow menu on phone')
    const fleet = page.getByTestId('fleet-open')
    await fleet.focus()
    await page.keyboard.press('Enter')
    await expect(page.getByRole('dialog')).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(fleet).toBeFocused()
  })

  test('visible text is at least 12px and controls have 24px hit areas', async ({ page }) => {
    await open(page)
    const { small, tiny } = await page.evaluate(() => {
      const small: string[] = []
      const tiny: string[] = []
      const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
      for (let n = walker.nextNode(); n; n = walker.nextNode()) {
        const el = n.parentElement
        if (!el || !n.textContent?.trim()) continue
        const r = el.getBoundingClientRect()
        if (r.width === 0 || r.height === 0 || el.closest('[aria-hidden="true"]')) continue
        if (parseFloat(getComputedStyle(el).fontSize) < 11.95)
          small.push(`${el.tagName}.${el.className}`.slice(0, 80))
      }
      for (const el of document.querySelectorAll('button, [role="tab"], a[href]')) {
        const r = el.getBoundingClientRect()
        if (r.width === 0 || r.height === 0 || el.classList.contains('sr-only')) continue
        if (r.width < 23.5 || r.height < 23.5)
          tiny.push(
            `${el.tagName}[${el.getAttribute('aria-label') ?? el.textContent.slice(0, 20)}]`,
          )
      }
      return { small, tiny }
    })
    expect(small).toEqual([])
    expect(tiny).toEqual([])
  })
})
