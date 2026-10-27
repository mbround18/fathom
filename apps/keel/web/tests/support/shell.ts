/**
 * Story helper: opens the signed-in shell against the fake backend. The us1–us9 placeholders use
 * it until each story writes its own specs on the `backend` fixture (fake-backend.ts).
 */
import { expect, type Page } from '@playwright/test'

import { FakeBackend } from './fake-backend'

/** Cookies ignore the port, so the host alone is enough for the session cookie. */
export async function openShell(page: Page, baseURL = 'http://localhost'): Promise<FakeBackend> {
  const backend = await FakeBackend.install(page, baseURL)
  await page.goto('/')
  await expect(page.getByRole('navigation', { name: 'Sections' })).toBeVisible()
  return backend
}
