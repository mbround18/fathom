/** Batch C (storage + config core): list rendering, Secret masking, confirmed deletes. */
import { expect, test } from '@playwright/test'

import { batch, clusterRoute, list, nsRoute, read } from '../fixtures/builders'
import { BATCH_C_LIST_EVENTS } from '../fixtures/batchCKinds'
import {
  check,
  clearSent,
  confirmDialog,
  emit,
  expectRowCount,
  navigateTo,
  open,
  row,
  sent,
} from './helpers'

const CASES = [
  { label: 'Persistent Volumes', group: 'Storage', section: 'storage/persistentvolumes', key: 'pv-data', cluster: true },
  { label: 'Persistent Volume Claims', group: 'Storage', section: 'storage/persistentvolumeclaims', key: 'default/data', cluster: false },
  { label: 'Storage Classes', group: 'Storage', section: 'storage/storageclasses', key: 'standard', cluster: true },
  { label: 'ConfigMaps', group: 'Config', section: 'config/configmaps', key: 'default/app-config', cluster: false },
  { label: 'Secrets', group: 'Config', section: 'config/secrets', key: 'default/db-creds', cluster: false },
  { label: 'Resource Quotas', group: 'Config', section: 'config/resourcequotas', key: 'default/compute', cluster: false },
  { label: 'Limit Ranges', group: 'Config', section: 'config/limitranges', key: 'default/defaults', cluster: false },
  { label: 'Pod Disruption Budgets', group: 'Config', section: 'config/poddisruptionbudgets', key: 'default/web-pdb', cluster: false },
] as const

for (const c of CASES) {
  test(`${c.label} lists rows`, async ({ page }) => {
    await open(page)
    await navigateTo(page, c.label, c.group)
    const route = c.cluster ? clusterRoute('kind-a', c.section) : nsRoute('kind-a', c.section)
    await emit(page, batch([list(1, route, BATCH_C_LIST_EVENTS[c.section])]))
    await expectRowCount(page, 1, c.section)
    await expect(row(page, c.key, c.section)).toBeVisible()
  })
}

test('Secrets show masked keys and never values', async ({ page }) => {
  const section = 'config/secrets'
  await open(page)
  await navigateTo(page, 'Secrets', 'Config')
  await emit(page, batch([list(1, nsRoute('kind-a', section), BATCH_C_LIST_EVENTS[section])]))
  await row(page, 'default/db-creds', section).click()
  await emit(
    page,
    batch([
      read(2, nsRoute('kind-a', section, 'default'), {
        type: 'SecretDetailsUpdated',
        data: {
          namespace: 'default',
          name: 'db-creds',
          creation_timestamp: '2025-01-01T00:00:00Z',
          labels: {},
          annotations: {},
          secret_type: 'Opaque',
          keys: ['username', 'password'],
          raw_json: '{}',
        },
      }),
    ]),
  )
  await expect(page.getByTestId('secret-keys').first()).toBeVisible()
  await expect(page.getByTestId('secret-key-username').first()).toContainText('username')
  await expect(page.getByTestId('secret-value-password').first()).toHaveText('••••••••')
  await page.getByRole('button', { name: 'Reveal value of password' }).first().click()
  await expect(page.getByTestId('secret-value-password').first()).toContainText(
    'Value not transferred',
  )
})

const DELETES = [
  {
    label: 'ConfigMaps',
    group: 'Config',
    section: 'config/configmaps',
    key: 'default/app-config',
    cmd: { type: 'DeleteConfigMap', data: { namespace: 'default', name: 'app-config' } },
  },
  {
    label: 'Persistent Volume Claims',
    group: 'Storage',
    section: 'storage/persistentvolumeclaims',
    key: 'default/data',
    cmd: { type: 'DeletePersistentVolumeClaim', data: { namespace: 'default', name: 'data' } },
  },
] as const

for (const d of DELETES) {
  test(`${d.label}: delete is confirmed and sends the exact command`, async ({ page }) => {
    await open(page)
    await navigateTo(page, d.label, d.group)
    await emit(
      page,
      batch([list(50, nsRoute('kind-a', d.section), BATCH_C_LIST_EVENTS[d.section])]),
    )
    await check(page, d.key, d.section)
    await clearSent(page)
    await page.getByTestId('bulk-delete').click()
    await expect(confirmDialog(page).getByTestId('confirm-item')).toHaveCount(1)
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect.poll(() => sent(page)).toEqual([d.cmd])
  })
}
