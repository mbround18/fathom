/** Batch C (storage + config core): list rendering, Secret masking, confirmed deletes. */
import { expect, test } from '@playwright/test'

import type { ResourceListEvent } from '@/protocol/generated/ResourceListEvent'
import type { SecretRevealOutcome } from '@/protocol/generated/SecretRevealOutcome'
import { batch, clusterRoute, list, nsRoute, read } from '../fixtures/builders'
import { BATCH_C_LIST_EVENTS } from '../fixtures/batchCKinds'
import {
  check,
  clearSent,
  confirmDialog,
  emit,
  expectRowCount,
  finish,
  navigateTo,
  open,
  requestIdOf,
  row,
  sent,
} from './helpers'

function ev(section: string): ResourceListEvent {
  const e = BATCH_C_LIST_EVENTS[section]
  if (!e) throw new Error(`no fixture for ${section}`)
  return e
}

const CASES = [
  {
    label: 'Persistent Volumes',
    group: 'Storage',
    section: 'storage/persistentvolumes',
    key: 'pv-data',
    cluster: true,
  },
  {
    label: 'Persistent Volume Claims',
    group: 'Storage',
    section: 'storage/persistentvolumeclaims',
    key: 'default/data',
    cluster: false,
  },
  {
    label: 'Storage Classes',
    group: 'Storage',
    section: 'storage/storageclasses',
    key: 'standard',
    cluster: true,
  },
  {
    label: 'ConfigMaps',
    group: 'Config',
    section: 'config/configmaps',
    key: 'default/app-config',
    cluster: false,
  },
  {
    label: 'Secrets',
    group: 'Config',
    section: 'config/secrets',
    key: 'default/db-creds',
    cluster: false,
  },
  {
    label: 'Resource Quotas',
    group: 'Config',
    section: 'config/resourcequotas',
    key: 'default/compute',
    cluster: false,
  },
  {
    label: 'Limit Ranges',
    group: 'Config',
    section: 'config/limitranges',
    key: 'default/defaults',
    cluster: false,
  },
  {
    label: 'Pod Disruption Budgets',
    group: 'Config',
    section: 'config/poddisruptionbudgets',
    key: 'default/web-pdb',
    cluster: false,
  },
] as const

for (const c of CASES) {
  test(`${c.label} lists rows`, async ({ page }) => {
    await open(page)
    await navigateTo(page, c.label, c.group)
    const route = c.cluster ? clusterRoute('kind-a', c.section) : nsRoute('kind-a', c.section)
    await emit(page, batch([list(1, route, ev(c.section))]))
    await expectRowCount(page, 1, c.section)
    await expect(row(page, c.key, c.section)).toBeVisible()
  })
}

test('Secrets show masked keys; Reveal fetches one value on demand', async ({ page }) => {
  const section = 'config/secrets'
  await open(page)
  await navigateTo(page, 'Secrets', 'Config')
  await emit(page, batch([list(1, nsRoute('kind-a', section), ev(section))]))
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
  // #40: exactly one key of this Secret is asked for, and only now.
  const reveal = (await sent(page)).filter((c) => c.type === 'RevealSecretValue')
  expect(reveal).toEqual([
    {
      type: 'RevealSecretValue',
      data: { namespace: 'default', name: 'db-creds', key: 'password' },
    },
  ])
  const id = await requestIdOf(page, 'RevealSecretValue')
  const answer = (outcome: SecretRevealOutcome) =>
    batch([
      read(
        3,
        nsRoute('kind-a', section, 'default'),
        {
          type: 'SecretValueRevealed',
          data: { namespace: 'default', name: 'db-creds', key: 'password', outcome },
        },
        { type: 'Request', data: id },
      ),
    ])
  await emit(page, answer({ type: 'Value', data: { value: 's3cr3t-pw' } }))
  await finish(page, id, true)
  await expect(page.getByTestId('secret-value-password').first()).toHaveText('s3cr3t-pw')
  await expect(page.getByRole('button', { name: 'Copy value of password' }).first()).toBeVisible()

  // Hide drops the value; a forbidden answer says so plainly.
  await page.getByRole('button', { name: 'Hide value of password' }).first().click()
  await expect(page.getByTestId('secret-value-password').first()).toHaveText('••••••••')
  await page.getByRole('button', { name: 'Reveal value of password' }).first().click()
  const id2 = await requestIdOf(page, 'RevealSecretValue')
  expect(id2).not.toBe(id)
  const denied = batch([
    read(
      4,
      nsRoute('kind-a', section, 'default'),
      {
        type: 'SecretValueRevealed',
        data: {
          namespace: 'default',
          name: 'db-creds',
          key: 'password',
          outcome: { type: 'Forbidden', data: { message: 'secrets "db-creds" is forbidden' } },
        },
      },
      { type: 'Request', data: id2 },
    ),
  ])
  await emit(page, denied)
  await finish(page, id2, true)
  await expect(page.getByTestId('secret-value-password').first()).toContainText('Forbidden')
  await expect(page.getByText('Value not transferred')).toHaveCount(0)
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
    await emit(page, batch([list(50, nsRoute('kind-a', d.section), ev(d.section))]))
    await check(page, d.key, d.section)
    await clearSent(page)
    await page.getByTestId('bulk-delete').click()
    await expect(confirmDialog(page).getByTestId('confirm-item')).toHaveCount(1)
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect.poll(() => sent(page)).toEqual([d.cmd])
  })
}
