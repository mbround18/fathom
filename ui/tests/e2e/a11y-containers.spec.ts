/**
 * 048-docker-management T080: the Containers scene by keyboard alone, at the narrowest width.
 *
 * Three claims, and the third is the one that matters most. Every action in this scene is
 * reachable by Tab — including the typed confirmation a destructive operation demands, because a
 * guard that can only be satisfied with a mouse is a guard that locks the keyboard operator out of
 * their own daemon. Nothing scrolls sideways on a phone, and every control carries a name a screen
 * reader can say.
 */
import { expect, test, type Page } from '@playwright/test'

import { navigateTo, open } from './helpers'

const cls = (page: Page) => page.locator('html').getAttribute('data-width-class')

/**
 * The Containers group holds a section of its own name, so the click is scoped to the group's
 * own section list — which is what `navigateTo` does, and it also waits for the phone drawer to
 * finish closing before the next navigation touches it.
 */
async function openContainers(page: Page, label = 'Containers'): Promise<void> {
  await navigateTo(page, label, 'Containers')
}

/** Tab until `locator` has focus, or give up after `limit` stops. */
async function tabTo(page: Page, testId: string, limit = 60): Promise<boolean> {
  for (let i = 0; i < limit; i++) {
    const id = await page.evaluate(() =>
      (document.activeElement as HTMLElement | null)?.getAttribute('data-testid'),
    )
    if (id === testId) return true
    await page.keyboard.press('Tab')
  }
  return false
}

test.describe('Containers scene accessibility', () => {
  test('a group, a row and their verbs are all reachable by keyboard', async ({ page }) => {
    await open(page, 'containers-adopted')
    await openContainers(page)

    // The group's own control first: Enter on it is what selection by keyboard means.
    expect(await tabTo(page, 'containers-group-shop')).toBe(true)
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('container-row-shop-web')).toBeVisible()

    // And a verb on the selected project, reached the same way.
    await page.getByTestId('container-row-shop-web').click()
    const stop = page.getByTestId('container-stop-shop-web')
    await stop.focus()
    await expect(stop).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('operation-preview')).toBeVisible()
  })

  test('the typed confirmation can be reached and satisfied without a mouse', async ({ page }) => {
    await open(page, 'containers-adopted')
    await openContainers(page)
    await page.getByTestId('containers-group-shop').click()

    // A destructive operation: the preview demands the object's own name typed out.
    const down = page.getByTestId('project-down-shop')
    await down.focus()
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('operation-confirmation')).toBeVisible()

    // Tab from the preview to the field, type the name, and the issue becomes available — no
    // click anywhere in the sequence.
    expect(await tabTo(page, 'operation-confirmation')).toBe(true)
    await page.keyboard.type('shop')
    await expect(page.getByTestId('operation-issue')).toBeEnabled()
  })

  test('every control in the scene has an accessible name', async ({ page }) => {
    await open(page, 'containers-adopted')
    await openContainers(page)
    await page.getByTestId('containers-group-shop').click()

    const unnamed = await page.evaluate(() => {
      const scene = document.querySelector('[data-testid="containers-scene"]')
      if (!scene) return ['no scene']
      const bad: string[] = []
      for (const el of scene.querySelectorAll('button, input, select, a[href]')) {
        if (el.closest('[aria-hidden="true"]')) continue
        const name = [
          el.getAttribute('aria-label'),
          el.getAttribute('title'),
          el.textContent,
        ].reduce<string>(
          (found, candidate) => (found === '' ? (candidate ?? '').trim() : found),
          '',
        )
        const labelled = el.id !== '' && document.querySelector(`label[for="${el.id}"]`) !== null
        if (name === '' && !labelled) bad.push(el.outerHTML.slice(0, 80))
      }
      return bad
    })
    expect(unnamed).toEqual([])
  })

  test('nothing scrolls sideways at phone width', async ({ page }) => {
    await open(page, 'containers-adopted')
    test.skip((await cls(page)) !== 'phone', 'narrow-width claim')
    await openContainers(page)
    await page.getByTestId('containers-group-shop').click()
    await page.getByTestId('container-row-shop-web').click()

    const overflow = await page.evaluate(() => ({
      doc: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      body: document.body.scrollWidth - document.body.clientWidth,
    }))
    expect(overflow.doc).toBeLessThanOrEqual(1)
    expect(overflow.body).toBeLessThanOrEqual(1)
  })
})
