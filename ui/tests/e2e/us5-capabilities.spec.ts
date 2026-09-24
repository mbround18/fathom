/**
 * T030 (033 US5): mutating controls follow the identity's real permissions. Allowed controls are
 * enabled, denied ones are disabled with the API server's reason, unverifiable checks fail closed.
 */
import { expect, test, type Page } from '@playwright/test'

import { genDeployments } from '../fixtures/gen-pods'
import { check, DEPLOYMENTS, open, row, showList } from './helpers'

const D = genDeployments('kind-a').find((d) => d.desired === 1)
if (!D) throw new Error('fixture: a deployment with 1 replica')
const KEY = `${D.namespace}/${D.name}`

interface FakeHandle {
  setCapabilities(config: unknown): void
}

async function configure(page: Page, config: unknown) {
  await page.evaluate((c) => {
    ;(window as unknown as { __fathomFake: FakeHandle }).__fathomFake.setCapabilities(c)
  }, config)
}

async function select(page: Page) {
  await showList(page, DEPLOYMENTS, 'Deployments')
  await expect(row(page, KEY, DEPLOYMENTS)).toBeVisible()
  await check(page, KEY, DEPLOYMENTS)
}

test.describe('US5 capability gating', () => {
  test('an allowed identity (alice) gets enabled controls', async ({ page }) => {
    await open(page)
    await select(page)
    await expect(page.getByTestId('bulk-scale')).toBeEnabled()
    await expect(page.getByTestId('bulk-delete')).toBeEnabled()
  })

  test('a denied identity (bob) sees the control disabled with the reason', async ({ page }) => {
    await open(page)
    await configure(page, {
      deny: [{ verb: 'delete', resource: 'deployments', reason: 'bob cannot delete deployments' }],
    })
    await select(page)
    const del = page.getByTestId('bulk-delete')
    await expect(del).toBeDisabled()
    await expect(del).toHaveAttribute('title', 'bob cannot delete deployments')
    await expect(page.getByTestId('bulk-scale')).toBeEnabled()
  })

  test('an unverifiable check fails closed with "could not verify"', async ({ page }) => {
    await open(page)
    await configure(page, { unverifiable: true })
    await select(page)
    const scale = page.getByTestId('bulk-scale')
    await expect(scale).toBeDisabled()
    await expect(scale).toHaveAttribute('title', /could not verify/i)
  })
})
