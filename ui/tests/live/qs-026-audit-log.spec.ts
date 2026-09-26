/**
 * LIVE quickstart validation for 026-mutation-audit-log-viewer (T036), US1 + US2, through the
 * devbridge against the real backend (which appends to the real `~/.fathom/audit-log.json`).
 *
 * Deviation from the quickstart: it cordons a node, but the devbridge refuses every
 * cluster-scoped write, so the success entries come from scaling `scale-me` 1 → 2 → 1. The failure
 * entry is a real backend failure: the UI deletes a throwaway pod, and the tap rewrites the
 * `DeletePod` to a name that does not exist, so the API server answers 404.
 *
 * The audit file is shared with every other run on this machine, so filter checks assert that
 * every visible entry matches, not exact counts.
 */
import type { Locator, Page } from '@playwright/test'

import {
  actionToast,
  checkRows,
  confirmDialog,
  createThrowawayPods,
  DEPLOYMENTS,
  expect,
  findRow,
  key,
  kubectl,
  LIVE_NAMESPACE,
  navigateTo,
  restoreBaseline,
  scaleDialog,
  showList,
  tapBridge,
  test,
  waitForPodsReady,
} from './helpers'

test.describe.configure({ timeout: 240_000 })

const MISSING = `qs026-missing-${Date.now().toString(36)}`

async function openAudit(page: Page): Promise<Locator> {
  await navigateTo(page, 'Audit Log', 'Team')
  await expect(page.getByTestId('audit-log-note')).toContainText('not Kubernetes')
  return page.getByTestId('audit-entry')
}

async function scale(page: Page, from: number, to: number): Promise<number> {
  await findRow(page, 'scale-me', DEPLOYMENTS)
  await checkRows(page, ['scale-me'], { section: DEPLOYMENTS })
  await page.getByTestId('bulk-scale').click()
  const dialog = scaleDialog(page)
  await expect(dialog.getByTestId('scale-current')).toHaveText(String(from))
  await dialog.getByLabel('Replicas', { exact: true }).fill(String(to))
  await dialog.getByTestId('scale-submit').click()
  await expect(
    actionToast(page, 'success').filter({ hasText: `Scaled deployment ${key('scale-me')}` }),
  ).toBeVisible({ timeout: 60_000 })
  return Date.now()
}

async function everyEntry(entries: Locator, check: (text: string) => boolean): Promise<void> {
  await expect(entries.first()).toBeVisible()
  const texts = await entries.allTextContents()
  const bad = texts.filter((t) => !check(t))
  expect(bad, `entries not matching the filter:\n${bad.join('\n')}`).toEqual([])
}

test.describe('live 026 quickstart', () => {
  test.afterAll(() => {
    restoreBaseline()
  })

  test('US1 + US2: real mutations are recorded with outcome, and filters narrow then clear', async ({
    page,
    app,
  }) => {
    kubectl(['scale', 'deployment/scale-me', '--replicas=1'])
    const [victim] = createThrowawayPods(1, 'qs026')
    if (!victim) throw new Error('no throwaway pod')
    await waitForPodsReady([victim])

    const tap = await tapBridge(page, (cmd) =>
      cmd.type === 'DeletePod' && cmd.data.name === victim
        ? { type: 'DeletePod', data: { namespace: LIVE_NAMESPACE, name: MISSING } }
        : cmd,
    )
    await app.open()
    await app.selectContext()
    await app.selectNamespace(LIVE_NAMESPACE)

    // --- US1: two successes and one failure.
    await showList(page, DEPLOYMENTS, 'Deployments')
    await scale(page, 1, 2)
    kubectl(['rollout', 'status', 'deployment/scale-me', '--timeout=120s'])
    await showList(page, DEPLOYMENTS, 'Deployments')
    await expect
      .poll(
        async () =>
          (await (
            await findRow(page, 'scale-me', DEPLOYMENTS)
          )
            .locator('[data-col="ready"]')
            .textContent()) ?? '',
        { timeout: 60_000 },
      )
      .toBe('2/2')
    const doneAt = await scale(page, 2, 1)

    let entries = await openAudit(page)
    const t0 = Date.now()
    await expect(entries.first()).toContainText('scale-me', { timeout: 10_000 })
    test.info().annotations.push({
      type: 'SC-001 ms (toast → entry, incl. navigation)',
      description: String(Date.now() - doneAt),
    })
    expect(Date.now() - t0).toBeLessThan(2_000 + 1_000)
    const top = entries.first()
    await expect(top).toContainText('kind-fathom-dev')
    await expect(top.getByTestId('audit-outcome')).toHaveText('Success')
    await expect(top.locator('time')).toBeVisible()
    await expect(entries.nth(1)).toContainText('scale-me')

    await navigateTo(page, 'Pods')
    await findRow(page, victim)
    await checkRows(page, [victim])
    await page.getByTestId('bulk-delete').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect(actionToast(page, 'error')).toBeVisible({ timeout: 60_000 })
    expect(tap.writes()).toContainEqual({
      type: 'DeletePod',
      data: { namespace: LIVE_NAMESPACE, name: MISSING },
    })

    entries = await openAudit(page)
    const failed = entries.first()
    await expect(failed).toContainText(MISSING, { timeout: 10_000 })
    await expect(failed.getByTestId('audit-outcome')).toHaveText('Failed')
    const failText = (await failed.textContent()) ?? ''
    test.info().annotations.push({ type: 'failure entry', description: failText })
    expect(failText).toMatch(/not ?found|404/i)

    // --- US2: filters.
    const scaleLabel = 'Scale' // AuditLogPane's ACTION_LABELS.scale
    await page.getByLabel('Action type').selectOption({ label: scaleLabel })
    await everyEntry(entries, (t) => t.includes(scaleLabel))
    await page.getByRole('button', { name: 'Clear filters' }).click()

    await page.getByLabel('Outcome').selectOption('failure')
    await everyEntry(entries, (t) => t.includes('Failed'))
    await expect(entries.filter({ hasText: MISSING })).toHaveCount(1)
    await page.getByRole('button', { name: 'Clear filters' }).click()

    await page.getByLabel('Search target name').fill('SCALE-M')
    await everyEntry(entries, (t) => t.toLowerCase().includes('scale-me'))
    const narrowed = await entries.count()
    await page.getByRole('button', { name: 'Clear filters' }).click()
    await expect.poll(() => entries.count()).toBeGreaterThan(narrowed)
    await expect(entries.filter({ hasText: MISSING })).toHaveCount(1)

    // US3.3: nothing here came from the API server's audit log.
    expect(tap.sent.some((c) => /cluster.?audit/i.test(c.type))).toBe(false)
  })
})
