/** 031 T051: audit actor column + filter appear only when entries carry an actor. */
import { expect, test } from '@playwright/test'

import type { AuditLogEntry } from '../../src/protocol/types'
import { batch, system } from '../fixtures/builders'
import { emit, navigateTo, open, sentTypes } from './helpers'
import { ACTOR_ENTRIES, PLAIN_ENTRY } from './hosted-audit-actor.fixtures'

async function load(page: Parameters<typeof open>[0], entries: AuditLogEntry[]) {
  await open(page)
  await navigateTo(page, 'Audit Log', 'Team')
  await expect.poll(() => sentTypes(page)).toContain('RefreshAuditLog')
  await emit(page, batch([system(1, { type: 'AuditLogLoaded', data: { entries } })]))
}

test('actor column and filter show for hosted entries and narrow by actor', async ({ page }) => {
  await load(page, ACTOR_ENTRIES)
  const entries = page.getByTestId('audit-entry')
  await expect(entries).toHaveCount(2)
  await expect(page.getByTestId('audit-actor').first()).toBeVisible()
  await page.getByLabel('Actor').selectOption('bob')
  await expect(entries).toHaveCount(1)
  await expect(entries.first().getByTestId('audit-actor')).toHaveText('bob')
})

test('actor column and filter are hidden when no entry has an actor', async ({ page }) => {
  await load(page, [PLAIN_ENTRY])
  await expect(page.getByTestId('audit-entry')).toHaveCount(1)
  await expect(page.getByTestId('audit-actor')).toHaveCount(0)
  await expect(page.getByLabel('Actor')).toHaveCount(0)
})
