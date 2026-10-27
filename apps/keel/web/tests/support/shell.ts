/**
 * Placeholder story helper (spec 055 T010): opens the shell. The fake backend replaces it in the
 * Foundational phase.
 */
import { expect, type Page } from '@playwright/test'

export async function openShell(page: Page): Promise<void> {
  await page.goto('/')
  await expect(page.getByRole('heading', { name: /Keel/ })).toBeVisible()
}
