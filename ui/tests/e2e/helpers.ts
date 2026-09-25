/**
 * Shared Playwright helpers for the US1 specs: drive the fake backend (`window.__fathomFake`)
 * and find panes and tables by their test hooks (contracts/layout.md § Test hooks).
 */
import { expect, type Locator, type Page, type TestInfo } from '@playwright/test'

import type { ScenarioName } from '../fixtures'
import type { BackendCmd, EventBatch } from '../../src/protocol/types'
import type {} from '../../src/protocol/fake-transport'

export const PODS = 'workloads/pods'
export const DEPLOYMENTS = 'workloads/deployments'

export function isPhone(info: TestInfo): boolean {
  return info.project.name.endsWith('-phone')
}

export async function open(page: Page, scenario: ScenarioName = 'default'): Promise<void> {
  await page.goto(`/?scenario=${scenario}`)
  await page.waitForFunction(() => window.__fathomFake !== undefined)
}

export async function emit(page: Page, batch: EventBatch): Promise<void> {
  await page.evaluate((b) => window.__fathomFake?.emit(b), batch)
}

export async function sent(page: Page, includePersistence = false): Promise<BackendCmd[]> {
  // Capability lookups (033 US5) are read-only background traffic, not part of any story's
  // command assertions. Neither is the debounced (500ms) `SaveUiConfig` layout/nav persistence,
  // which lands at an arbitrary point after nav-group expansion or a layout change and would
  // otherwise race every exact-match command assertion. Specs that assert persistence itself
  // opt in with `includePersistence`.
  const all = await page.evaluate(() => window.__fathomFake?.sent() ?? [])
  return all.filter(
    (c) =>
      c.type !== 'ResolveCapabilities' &&
      c.type !== 'CheckCapability' &&
      (includePersistence || c.type !== 'SaveUiConfig'),
  )
}

/**
 * Commands sent excluding `SaveUiConfig`: the nav-group expand persist is debounced, so it can land
 * after a test's `clearSent` and is unrelated to the mutation flows under assertion.
 */
export async function sentMutations(page: Page): Promise<BackendCmd[]> {
  return (await sent(page)).filter((c) => c.type !== 'SaveUiConfig')
}

/**
 * Wait for the fake scenario's own async replies (queued by `setTimeout` at `send` time) to be
 * delivered. Timers with equal delay fire FIFO, so a page-side `setTimeout(0)` queued now lands
 * after them; without this an injected `emit` can be overwritten by a late canned reply.
 */
export async function settleFakeReplies(page: Page): Promise<void> {
  await page.evaluate(() => new Promise<void>((resolve) => setTimeout(resolve, 0)))
}

export async function clearSent(page: Page): Promise<void> {
  await page.evaluate(() => window.__fathomFake?.clearSent())
}

/** Command `type`s sent since the last `clearSent`. */
export async function sentTypes(page: Page, includePersistence = false): Promise<string[]> {
  return (await sent(page, includePersistence)).map((c) => c.type)
}

/** The resource table showing `section` (the first, if several panes show it). */
export function table(page: Page, section: string = PODS): Locator {
  return page.locator(`[data-testid="resource-table"][data-section="${section}"]`).first()
}

/** The list pane host showing `section` (whatever state it is in). */
export function listPane(page: Page, section: string = PODS): Locator {
  return page
    .locator(`[data-slot="pane"][data-pane-kind="list"][data-section="${section}"]`)
    .first()
}

export function searchBox(page: Page, section: string = PODS): Locator {
  return listPane(page, section).getByRole('searchbox')
}

export function inspector(page: Page): Locator {
  return page.locator('[data-slot="pane"][data-pane-kind="inspector"]').first()
}

export function row(page: Page, key: string, section: string = PODS): Locator {
  return table(page, section).locator(`[role="row"][data-row-key="${key}"]`)
}

/** Wait until the table for `section` reports `count` rows (after filtering). */
export async function expectRowCount(page: Page, count: number, section = PODS): Promise<void> {
  await expect(table(page, section)).toHaveAttribute('data-row-count', String(count))
}

/** Open `section` from the navigator (the drawer on phone/tablet, the nav slot otherwise). */
export async function navigateTo(page: Page, label: string, group = 'Workloads'): Promise<void> {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  const drawer = !(await nav.isVisible())
  if (drawer) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav).toBeVisible()
  // Scoped to `[aria-expanded]` (only the group toggle has it) since a group's own section can
  // share its exact label (e.g. the "Overview" group contains an "Overview" section), which
  // would otherwise make `getByRole('button', { name: group, exact: true })` ambiguous once the
  // group is already expanded and both buttons are in the DOM.
  const toggle = nav.locator('button[aria-expanded]', { hasText: group })
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
  await nav.getByRole('button', { name: label, exact: true }).click()
  // Let the drawer finish closing so a following call does not race its exit animation.
  if (drawer) await expect(nav).toBeHidden()
}

/** No horizontal page scroll (contracts/layout.md § Verification). */
export async function expectNoHorizontalScroll(page: Page): Promise<void> {
  const { scrollWidth, clientWidth } = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }))
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth)
}

// ---- US3 actions

/** The `RequestId` the fake will assign to the next command. */
export async function nextRequestId(page: Page): Promise<number> {
  return page.evaluate(() => window.__fathomFake?.nextRequestId() ?? -1)
}

/**
 * The `RequestId` the fake actually assigned to the latest sent command of `type`. Poll it after
 * the click that sends the command; predicting via `nextRequestId` races background commands.
 */
export async function requestIdOf(page: Page, type: BackendCmd['type']): Promise<number> {
  let id: number | undefined
  await expect
    .poll(async () => {
      id = await page.evaluate((t) => window.__fathomFake?.requestIdOf(t), type)
      return id
    })
    .toBeDefined()
  return id ?? -1
}

/** Deliver `CommandFinished` for `requestId`, as the backend does when the command completes. */
export async function finish(
  page: Page,
  requestId: number,
  ok: boolean,
  error?: string,
): Promise<void> {
  await page.evaluate(([id, o, e]) => window.__fathomFake?.finish(id, o, e), [
    requestId,
    ok,
    error,
  ] as const)
}

/** Tick a row's checkbox (multi-select). */
export async function check(page: Page, key: string, section: string = PODS): Promise<void> {
  const box = row(page, key, section).getByRole('checkbox', { name: `Select ${key}` })
  await box.click()
  await expect(box).toHaveAttribute('aria-checked', 'true')
}

/** Make `section`'s list visible (wide/ultra show deployments beside pods already). */
export async function showList(page: Page, section: string, label: string): Promise<void> {
  if (!(await table(page, section).isVisible())) await navigateTo(page, label)
  await expect(table(page, section)).toBeVisible()
}

/** The result toast of the latest action with outcome `type` (sonner's `data-type`). */
export function actionToast(page: Page, type: 'success' | 'error' | 'loading'): Locator {
  return page.locator(`[data-testid="action-toast"][data-type="${type}"]`)
}

export const confirmDialog = (page: Page): Locator => page.getByTestId('confirm-dialog')
export const scaleDialog = (page: Page): Locator => page.getByTestId('scale-dialog')

/**
 * Press Delete on the focused row/selection and wait for the confirm dialog. The key bypasses the
 * gated menu item and is refused (while the capability check is requested) until the backend's
 * verified "allowed" answer has landed, so a press right after selecting can be a no-op; retry.
 */
export async function pressDelete(page: Page): Promise<void> {
  await expect(async () => {
    await page.keyboard.press('Delete')
    await expect(page.getByTestId('confirm-dialog')).toBeVisible({ timeout: 500 })
  }).toPass({ timeout: 10_000 })
}

/** Phone collapses the context/namespace pickers into a chip that opens a sheet. */
async function onPhone(page: Page): Promise<boolean> {
  return (await page.locator('html').getAttribute('data-width-class')) === 'phone'
}

/** Pick `option` in the Context/Namespace picker, opening the scope sheet on phone. */
export async function chooseScope(
  page: Page,
  picker: 'Context' | 'Namespace',
  option: string,
): Promise<void> {
  const phone = await onPhone(page)
  if (phone) await page.getByTestId('scope-chip').click()
  await page.getByRole('combobox', { name: picker }).click()
  await page.getByRole('option', { name: option }).click()
  if (phone) await page.keyboard.press('Escape')
}

/** Assert the Context/Namespace picker currently shows `text` (via the chip on phone). */
export async function expectScope(
  page: Page,
  picker: 'Context' | 'Namespace',
  text: string,
): Promise<void> {
  if (await onPhone(page)) await expect(page.getByTestId('scope-chip')).toContainText(text)
  else await expect(page.getByRole('combobox', { name: picker })).toContainText(text)
}

/** Open a tool pane from whichever surface the width class offers. */
export async function openToolPane(page: Page, label: string): Promise<void> {
  const cls = await page.locator('html').getAttribute('data-width-class')
  if (cls === 'phone') {
    const views = page.getByRole('navigation', { name: 'Views' })
    const direct = views.getByRole('button', { name: label, exact: true })
    if (await direct.count()) await direct.click()
    else {
      await views.getByRole('button', { name: 'More' }).click()
      await page.getByTestId('more-sheet').getByRole('button', { name: label, exact: true }).click()
    }
  } else if (cls === 'tablet') {
    await page
      .getByRole('navigation', { name: 'Views' })
      .getByRole('button', { name: label })
      .click()
  } else if (cls === 'laptop') {
    await page.getByTestId('tools-menu').click()
    await page.getByRole('menuitem', { name: label }).click()
  } else {
    await page.getByRole('tab', { name: label }).click()
  }
}

/** Open the fleet dashboard (a top-bar button, or an overflow-menu item on phone). */
export async function openFleet(page: Page): Promise<void> {
  const button = page.getByTestId('fleet-open')
  if (await button.isVisible()) {
    await button.click()
    return
  }
  await page.getByTestId('overflow-menu').click()
  await page.getByRole('menuitem', { name: 'Fleet' }).click()
}

/** Open the fake backend presenting as a hosted shell (031-hosted-server-mode). */
export async function openHosted(page: Page, qs = 'user=alice&groups=dev,ops'): Promise<void> {
  await page.goto(`/?scenario=default&mode=hosted&${qs}`)
  await page.waitForFunction(() => window.__fathomFake !== undefined)
}
