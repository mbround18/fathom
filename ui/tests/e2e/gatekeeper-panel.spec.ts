/**
 * 020-gatekeeper-policy-pane: two scenarios against the scripted fake backend.
 *  A. an Active overview renders its violation and clicking the violating resource opens the
 *     Inspector on that exact object;
 *  B. NotInstalled renders the explicit "not installed" message.
 */
import { expect, test } from '@playwright/test'

import type { GatekeeperOverview } from '../../src/protocol/types'
import { batch, system } from '../fixtures/builders'
import { clearSent, emit, inspector, navigateTo, open, sent } from './helpers'

async function openGatekeeper(page: Parameters<typeof open>[0]) {
  await open(page)
  await navigateTo(page, 'Gatekeeper', 'Integrations')
}

test.describe('Gatekeeper policy pane', () => {
  test('renders a violation and opens the Inspector on the violating resource', async ({
    page,
  }) => {
    await openGatekeeper(page)
    const overview: GatekeeperOverview = {
      state: 'Active',
      error: '',
      constraints: [
        {
          kind: 'K8sRequiredLabels',
          name: 'require-team',
          enforcement_action: 'deny',
          total_violations: 1,
          violations: [
            {
              section: 'workloads/pods',
              kind: 'Pod',
              namespace: 'default',
              name: 'nginx-gk',
              enforcement_action: 'deny',
              message: 'missing label team',
            },
          ],
          violations_truncated: false,
          template_missing: false,
        },
      ],
    }
    await emit(page, batch([system(1, { type: 'GatekeeperOverviewUpdated', data: overview })]))

    await expect(page.getByText(/1 violation/).first()).toBeVisible()
    await page.getByTestId('gatekeeper-constraint').click()
    const violation = page.getByTestId('gatekeeper-violation')
    await expect(violation).toContainText('default/nginx-gk')

    await clearSent(page)
    await violation.click()
    await expect(inspector(page)).toBeVisible()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'SelectPod'))
      .toEqual({ type: 'SelectPod', data: { namespace: 'default', name: 'nginx-gk' } })
  })

  test('shows an explicit not-installed message', async ({ page }) => {
    await openGatekeeper(page)
    await emit(
      page,
      batch([
        system(1, {
          type: 'GatekeeperOverviewUpdated',
          data: { state: 'NotInstalled', error: '', constraints: [] },
        }),
      ]),
    )
    await expect(page.getByText(/OPA Gatekeeper is not installed on this cluster/)).toBeVisible()
  })
})
