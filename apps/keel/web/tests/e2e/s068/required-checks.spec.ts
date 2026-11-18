/**
 * Spec 068 T060 (US7, contracts/ui.md "Binding page", "Runs list"): an admin requires forge status
 * contexts on a binding, removes them and sets the timeout, each through `PreviewRequiredChecks`
 * and the shared Confirm (step-up); a forge that can't report statuses refuses and the section says
 * so; an operator only reads them. A run waiting on its checks lists each with its state and forge
 * link, live from `RunChecksChanged`. At 360 px and 1280 px, keyboard only.
 */
import type { Page } from '@playwright/test'

import { expect, test } from '../../support/fake-backend'
import { runInfo } from '../../support/fake-runs'
import { URL } from '../s066/repo-setup'
import { running } from './runs-setup'
import { press, WIDTHS } from './setup'

async function openBinding(page: Page) {
  await page.goto('/bindings')
  await press(page, page.getByRole('button', { name: URL }).first())
  const section = page.locator('[data-required-checks]')
  await expect(section).toBeVisible()
  return section
}

async function confirm(page: Page) {
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Require')
  await press(page, dialog.getByRole('button', { name: 'Confirm' }))
  await expect(dialog).toBeHidden()
}

for (const size of WIDTHS) {
  test.describe(size.name, () => {
    test.use({ viewport: { width: size.width, height: size.height } })

    test('an admin requires a check, then removes one, previewed and confirmed', async ({ page, backend }) => {
      running(backend, { role: 'ws_admin' })
      const section = await openBinding(page)
      await expect(section).toContainText('None: runs start without waiting on the forge.')

      await section.getByLabel('Status context').fill('ci/build')
      await press(page, section.getByRole('button', { name: 'Require' }))
      expect(await backend.waitForCmd('PreviewRequiredChecks')).toMatchObject({
        data: { binding_id: 'binding-ops', contexts: ['ci/build'] },
      })
      await confirm(page)
      await expect(section.locator('[data-required-check="ci/build"]')).toBeVisible()
      await expect(section).toContainText('Runs wait up to 1 h')

      await section.getByLabel('Status context').fill('ci/lint')
      await press(page, section.getByLabel('Status context'))
      expect(await backend.waitForCmd('PreviewRequiredChecks')).toMatchObject({ data: { contexts: ['ci/build', 'ci/lint'] } })
      await confirm(page)
      await expect(section.locator('[data-required-check]')).toHaveCount(2)

      await press(page, section.getByRole('button', { name: 'Stop requiring ci/build' }))
      expect(await backend.waitForCmd('PreviewRequiredChecks')).toMatchObject({ data: { contexts: ['ci/lint'] } })
      await confirm(page)
      await expect(section.locator('[data-required-check]')).toHaveCount(1)
    })

    test('an admin sets how long runs wait', async ({ page, backend }) => {
      running(backend, { role: 'ws_admin' })
      const b = backend.repos.bindings[0]
      if (b) b.required_checks = [{ context: 'ci/build' }]
      const section = await openBinding(page)
      const minutes = section.getByLabel('Wait at most (minutes)')
      await expect(minutes).toHaveValue('60')
      await expect(section.getByRole('button', { name: 'Save' })).toBeDisabled()
      await minutes.fill('30')
      await press(page, section.getByRole('button', { name: 'Save' }))
      expect(await backend.waitForCmd('PreviewRequiredChecks')).toMatchObject({
        data: { contexts: ['ci/build'], timeout_secs: 1800 },
      })
      await confirm(page)
      await expect(section).toContainText('Runs wait up to 30 min')
    })

    test('a forge without commit statuses refuses, and the section says why', async ({ page, backend }) => {
      const { r068 } = running(backend, { role: 'ws_admin' })
      r068.refuse = { code: 'checks_unsupported', message: 'This forge cannot report commit statuses, so runs cannot wait on them.' }
      const section = await openBinding(page)
      await section.getByLabel('Status context').fill('ci/build')
      await press(page, section.getByRole('button', { name: 'Require' }))
      await expect(section.getByRole('alert')).toHaveText('This forge cannot report commit statuses, so runs cannot wait on them.')
      await expect(section.getByLabel('Status context')).toBeDisabled()
      await expect(section.getByRole('button', { name: 'Require' })).toBeDisabled()
    })

    test('an operator reads the required checks but cannot change them', async ({ page, backend }) => {
      running(backend, { role: 'operator' })
      const b = backend.repos.bindings[0]
      if (b) b.required_checks = [{ context: 'ci/build' }]
      await page.goto('/bindings')
      const open = page.getByRole('button', { name: URL }).first()
      await press(page, open)
      const section = page.locator('[data-required-checks]')
      await expect(section.locator('[data-required-check="ci/build"]')).toBeVisible()
      await expect(section.getByRole('button')).toHaveCount(0)
      await expect(section.getByLabel('Status context')).toHaveCount(0)
    })

    test('a run waiting on its checks shows each with its state and forge link, live', async ({ page, backend }) => {
      const { runs, r068 } = running(backend)
      const ws = backend.me?.grants[0]?.workspace ?? ''
      runs.add(
        runInfo({
          id: 'run-wait',
          workspace: ws,
          job_name: 'debian',
          state: 'waiting_checks',
          started_at: null,
          checks: [
            { context: 'ci/build', state: 'pending', target_url: 'https://ci.example.com/build/7' },
            { context: 'ci/lint', state: 'missing', target_url: null },
          ],
        }),
      )
      await page.goto('/runs/run-wait')
      const checks = page.locator('[data-run-checks]')
      await expect(checks).toHaveAttribute('data-run-checks', 'waiting')
      await expect(page.locator('[data-run-state]').first()).toContainText('waiting for checks')
      await expect(checks).toContainText('0 of 2 required checks passed')
      await expect(checks.locator('[data-check="ci/lint"]')).toContainText('not reported yet')
      const link = checks.locator('[data-check="ci/build"]').getByRole('link', { name: /details/ })
      await expect(link).toHaveAttribute('href', 'https://ci.example.com/build/7')
      await expect(page.getByRole('button', { name: 'Cancel run' })).toBeVisible()

      r068.checks('run-wait', [
        { context: 'ci/build', state: 'success', target_url: 'https://ci.example.com/build/7' },
        { context: 'ci/lint', state: 'failure', target_url: 'https://ci.example.com/lint/7' },
      ])
      await expect(checks.locator('[data-check="ci/build"]')).toHaveAttribute('data-check-state', 'success')
      await expect(checks.locator('[data-check="ci/lint"]')).toContainText('failed')
      await expect(checks).toContainText('1 of 2 required checks failed')
    })
  })
}
