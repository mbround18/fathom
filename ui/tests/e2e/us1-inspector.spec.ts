/**
 * T056 (US1 scenario 4): selecting a pod reads it, and the inspector shows metadata, status,
 * spec, labels/annotations, YAML, and events. A later list without the object shows
 * NoLongerExists. Also covers the keyboard contract (arrows move, Enter opens, Esc closes).
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD, PODS_500, pods } from '../fixtures'
import {
  clearSent,
  emit,
  expectNoHorizontalScroll,
  expectRowCount,
  inspector,
  isPhone,
  open,
  row,
  sent,
  table,
} from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

test.describe('US1 inspector', () => {
  test('selecting a pod reads it and shows every detail section, YAML, and events', async ({
    page,
  }) => {
    await open(page)
    await expectRowCount(page, 500)
    await clearSent(page)

    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toEqual(expect.arrayContaining(['SelectPod', 'RefreshEvents']))
    const select = (await sent(page)).find((c) => c.type === 'SelectPod')
    expect(select).toEqual({
      type: 'SelectPod',
      data: { namespace: DETAIL_POD.namespace, name: DETAIL_POD.name },
    })

    await expect(insp.getByRole('heading', { name: DETAIL_POD.name })).toBeVisible()
    for (const h of ['Metadata', 'Status', 'Spec', 'Labels', 'Annotations']) {
      await expect(insp.getByRole('heading', { name: h, exact: true })).toBeVisible()
    }
    await expect(insp).toContainText('ReplicaSet')
    await expect(insp).toContainText('pod-template-hash')

    await insp.getByRole('tab', { name: 'YAML' }).click()
    const yaml = insp.locator('pre')
    await expect(yaml).toContainText('kind: Pod')
    await expect(yaml).toContainText(`name: ${DETAIL_POD.name}`)

    await insp.getByRole('tab', { name: 'Events' }).click()
    await expect(insp).toContainText('BackOff')
    await expect(insp).toContainText('Scheduled')

    await expectNoHorizontalScroll(page)
  })

  test('shows NoLongerExists when a refresh drops the selected pod', async ({ page }) => {
    await open(page)
    await expectRowCount(page, 500)
    await row(page, key(DETAIL_POD)).click()
    await expect(
      inspector(page).getByRole('heading', { name: 'Metadata', exact: true }),
    ).toBeVisible()

    await emit(
      page,
      pods(
        'kind-a',
        PODS_500.filter((p) => p !== DETAIL_POD),
        99,
      ),
    )

    await expect(inspector(page).locator('[data-pane-state="gone"]')).toBeVisible()
    await expect(inspector(page)).toContainText(DETAIL_POD.name)
  })

  test('arrows move the cursor, Enter opens the inspector, Esc closes it', async ({
    page,
  }, info) => {
    await open(page)
    await expectRowCount(page, 500)
    const second = PODS_500[1]
    if (!second) throw new Error('fixture')

    await table(page).getByRole('grid').focus()
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('ArrowDown')
    await expect(row(page, key(second))).toHaveAttribute('data-focused', 'true')
    await page.keyboard.press('Enter')

    await expect(inspector(page).getByRole('heading', { name: second.name })).toBeVisible()
    if (!isPhone(info)) {
      await expect(row(page, key(second))).toHaveAttribute('aria-selected', 'true')
    }

    await page.keyboard.press('Escape')
    if (isPhone(info)) {
      await expect(inspector(page)).toHaveCount(0)
      await expect(table(page)).toBeVisible()
    } else {
      await expect(inspector(page).getByRole('heading', { name: second.name })).toHaveCount(0)
    }
  })
})
