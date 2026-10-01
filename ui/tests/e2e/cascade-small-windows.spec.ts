/**
 * 044 US3 (T030): the cascade is a wide-and-above feature only. Phone, tablet, laptop and desktop
 * keep exactly today's behaviour (FR-009, SC-005): no cascade markup reaches them, a section click
 * leaves the inspector waiting for the operator's own click (nothing is auto-opened below wide),
 * and the phone still drills in as a stack.
 */
import { expect, test, type Page } from '@playwright/test'

import { widthClass } from './layout-helpers'
import {
  DEPLOYMENTS,
  clearSent,
  expectRowCount,
  inspector,
  isPhone,
  navigateTo,
  open,
  sentTypes,
  table,
} from './helpers'

/** Every hook the cascade contract adds (contracts/ui-cascade.md § Slots and test ids). */
const CASCADE_HOOKS = ['[data-testid^="cascade-"]', '[data-auto-open]', '[data-step]'] as const

const SMALL = ['phone', 'tablet', 'laptop', 'desktop']

/** Skip on wide and above: this spec is the below-wide regression gate. */
async function onlySmall(page: Page): Promise<void> {
  const cls = await widthClass(page)
  test.skip(!SMALL.includes(cls ?? ''), 'below wide only')
}

async function expectNoCascadeMarkup(page: Page): Promise<void> {
  for (const hook of CASCADE_HOOKS) await expect(page.locator(hook)).toHaveCount(0)
}

test.describe('US3 small windows are untouched by the cascade', () => {
  test('no cascade test id is present at any point of a drill-in', async ({ page }) => {
    await open(page)
    await onlySmall(page)
    await expectRowCount(page, 500)
    await expectNoCascadeMarkup(page)

    await navigateTo(page, 'Deployments')
    await expect(table(page, DEPLOYMENTS)).toBeVisible()
    await expectNoCascadeMarkup(page)

    const first = table(page, DEPLOYMENTS).locator('[role="row"][data-row-key]').first()
    const key = await first.getAttribute('data-row-key')
    await first.click()
    await expect(inspector(page)).toBeVisible()
    await expectNoCascadeMarkup(page)
    expect(key).toBeTruthy()
  })

  test('a section click leaves the inspector waiting for the operator’s click', async ({
    page,
  }, info) => {
    await open(page)
    await onlySmall(page)
    // The phone has no inspector slot at all until a row is tapped; it is covered below.
    test.skip(isPhone(info), 'phone has no inspector slot')
    await expectRowCount(page, 500)

    await clearSent(page)
    await navigateTo(page, 'Deployments')
    await expect(table(page, DEPLOYMENTS)).toBeVisible()

    // FR-003: below wide nothing is auto-opened — the pane still shows its empty state, and no
    // read command for a row was emitted by the section click.
    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect(insp).toContainText('Nothing selected')
    // Every per-object read is a `Select<Kind>` command, so none of them having been sent is the
    // command-level proof that no row was opened behind the operator's back.
    expect((await sentTypes(page)).filter((t) => t.startsWith('Select'))).toEqual([])

    // The operator's own click is what fills it, as it does today.
    const first = table(page, DEPLOYMENTS).locator('[role="row"][data-row-key]').first()
    const key = (await first.getAttribute('data-row-key')) ?? ''
    await first.click()
    await expect(insp).toContainText(key.split('/').pop() ?? '')
    await expectNoCascadeMarkup(page)
  })

  test('the phone stack still pushes and pops', async ({ page }, info) => {
    await open(page)
    test.skip(!isPhone(info), 'phone only')
    await expectRowCount(page, 500)

    const first = table(page).locator('[role="row"][data-row-key]').first()
    const key = (await first.getAttribute('data-row-key')) ?? ''
    await first.click()

    // Push: the inspector takes the whole screen and the list is gone.
    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect(insp).toContainText(key.split('/').pop() ?? '')
    await expect(table(page)).toHaveCount(0)
    await expectNoCascadeMarkup(page)

    // Pop: Back returns to the list.
    await page.getByRole('button', { name: 'Back', exact: true }).click()
    await expect(table(page)).toBeVisible()
    await expect(inspector(page)).toHaveCount(0)
    await expectNoCascadeMarkup(page)
  })
})
