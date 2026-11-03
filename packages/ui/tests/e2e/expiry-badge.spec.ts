/** ExpiryBadge: when something expires, in words and an icon as well as colour. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'expiry-badge'

test('tones: each badge says when, with its tone and the exact time in the title', async ({ page }) => {
  await openPiece(page, { piece, state: 'tones' })
  const badges = page.locator('[data-expiry]')
  await expect(badges).toHaveText(['Expires in 200 days', 'Expires in 21 days', 'Expires tomorrow', 'Expired 2 days ago'])
  expect(await badges.evaluateAll((els) => els.map((e) => e.getAttribute('data-expiry')))).toEqual([
    'ok',
    'soon',
    'urgent',
    'expired',
  ])
  await expect(badges.nth(1)).toHaveAttribute('title', '2026-11-23 12:00 UTC')
  await expect(page.getByText('Expires unknown')).toBeVisible()
  // Each tone has its own icon, so colour is never the only signal.
  expect(await badges.evaluateAll((els) => new Set(els.map((e) => e.querySelector('svg')?.innerHTML)).size)).toBe(4)
})

test('tones: no accessibility violations', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'tones' })
})
