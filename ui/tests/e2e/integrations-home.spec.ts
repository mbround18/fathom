/**
 * 043 US7 (T070): integrations live in one place. With cert-manager installed and Velero,
 * Knative and KEDA absent, Certificates is a normal entry, Knative and KEDA are greyed with
 * "Not installed in this cluster", and Velero appears nowhere (navigator, Tools, tablet tabs,
 * palette) until it is detected. Nothing offers to install an integration.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { VELERO_HEALTHY_AND_FAILED, VELERO_NOT_INSTALLED } from '../fixtures/velero'
import { emit, open } from './helpers'

const nav = (page: Page) => page.getByRole('navigation', { name: 'Sections' })

async function showIntegrations(page: Page): Promise<void> {
  if (!(await nav(page).isVisible())) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav(page)).toBeVisible()
  const toggle = nav(page).locator(':scope > div > button[aria-expanded]', {
    hasText: 'Integrations',
  })
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
}

async function closeNav(page: Page): Promise<void> {
  if (await page.getByRole('dialog').isVisible()) await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog')).toHaveCount(0)
}

/** cert-manager present; Velero, Knative and KEDA absent. */
async function detect(page: Page, velero = VELERO_NOT_INSTALLED, seq = 1): Promise<void> {
  await emit(
    page,
    batch([
      system(seq, {
        type: 'CertManagerOverviewUpdated',
        data: { installed: true, certificates: [] },
      }),
      system(seq + 1, {
        type: 'KnativeOverviewUpdated',
        data: { installed: false, services: [], revisions: [], routes: [] },
      }),
      system(seq + 2, {
        type: 'KedaOverviewUpdated',
        data: { installed: false, scaled_objects: [], scaled_jobs: [] },
      }),
      system(seq + 3, { type: 'VeleroOverviewUpdated', data: velero }),
    ]),
  )
}

/** The tool names each width class lists outside the navigator. */
async function toolNames(page: Page): Promise<string[]> {
  const cls = await page.locator('html').getAttribute('data-width-class')
  if (cls === 'phone') {
    await page
      .getByRole('navigation', { name: 'Views' })
      .getByRole('button', { name: 'More' })
      .click()
    const names = await page.getByTestId('more-sheet').getByRole('button').allInnerTexts()
    await page.keyboard.press('Escape')
    return names
  }
  if (cls === 'tablet') {
    await expect(page.getByRole('navigation', { name: 'Views' })).toBeVisible()
    return page.getByRole('navigation', { name: 'Views' }).getByRole('button').allInnerTexts()
  }
  if (cls === 'laptop') {
    await page.getByTestId('tools-menu').click()
    const names = await page.getByRole('menuitem').allInnerTexts()
    await page.keyboard.press('Escape')
    return names
  }
  // Desktop and up: the navigator's Tools list.
  return nav(page).locator('button[data-tool]').allInnerTexts()
}

async function paletteFinds(page: Page, query: string): Promise<number> {
  await page.getByTestId('palette-open').click()
  await page.getByPlaceholder(/Jump to a resource/).fill(query)
  const options = page.getByRole('option')
  await expect(page.getByRole('dialog')).toBeVisible()
  // Let the ranking settle before counting.
  await page.waitForTimeout(150)
  const n = await options.filter({ hasText: /velero/i }).count()
  await page.keyboard.press('Escape')
  return n
}

test.describe('Integrations home (US7)', () => {
  test('installed is normal, absent is greyed with the hint, Velero is hidden', async ({
    page,
  }) => {
    await open(page)
    await showIntegrations(page)
    await detect(page)

    const entry = (section: string) => nav(page).locator(`button[data-section="${section}"]`)
    await expect(entry('integrations/cert-manager')).toBeVisible()
    await expect(entry('integrations/cert-manager')).not.toHaveAttribute('data-installed', 'false')
    for (const section of ['integrations/knative', 'integrations/keda']) {
      await expect(entry(section)).toHaveAttribute('data-installed', 'false')
      await expect(entry(section)).toHaveAttribute('title', /Not installed in this cluster/)
      await expect(entry(section)).toHaveAccessibleDescription(/Not installed in this cluster/)
    }
    await expect(entry('integrations/velero')).toHaveCount(0)
    await expect(nav(page).getByText('Velero', { exact: true })).toHaveCount(0)

    // No entry anywhere in the navigator offers to install anything.
    await expect(nav(page).getByRole('button', { name: /install/i })).toHaveCount(0)
    await closeNav(page)

    expect((await toolNames(page)).join('|')).not.toMatch(/velero/i)
    expect(await paletteFinds(page, 'velero')).toBe(0)
  })

  test('Velero appears once it is detected', async ({ page }) => {
    await open(page)
    await showIntegrations(page)
    await detect(page)
    await expect(nav(page).locator('button[data-section="integrations/velero"]')).toHaveCount(0)

    await detect(page, VELERO_HEALTHY_AND_FAILED, 10)
    const velero = nav(page).locator('button[data-section="integrations/velero"]')
    await expect(velero).toBeVisible()
    await expect(velero).not.toHaveAttribute('data-installed', 'false')
    await closeNav(page)

    const cls = await page.locator('html').getAttribute('data-width-class')
    // Desktop and up list tools only through the navigator's short Tools list (logs, topology,
    // linter); Velero's home there is Integrations, asserted above.
    if (cls === 'phone' || cls === 'tablet' || cls === 'laptop') {
      expect((await toolNames(page)).join('|')).toMatch(/velero/i)
    }
    expect(await paletteFinds(page, 'velero')).toBeGreaterThan(0)
  })
})
