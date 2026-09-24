/** 023-cert-manager-certificates-pane: list + expiry, chain drill-down, honest empty state. */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { clearSent, emit, open, sent } from './helpers'

async function openCertificates(page: Page, info: TestInfo): Promise<void> {
  // The laptop class has neither a dock nor a tab strip, so no tool pane is reachable there.
  test.skip(info.project.name.endsWith('-laptop'), 'no tool-pane surface at laptop width')
  const stacked = info.project.name.endsWith('-phone') || info.project.name.endsWith('-tablet')
  if (stacked) {
    await page
      .getByRole('navigation', { name: 'Views' })
      .getByRole('button', { name: 'Certificates' })
      .click()
  } else {
    await page.getByRole('tab', { name: 'Certificates' }).click()
  }
}

const base = {
  namespace: 'default',
  issuer_ref: 'ClusterIssuer/letsencrypt',
  message: '',
  renewal_time: null,
  dns_names: ['example.com'],
}

test.describe('cert-manager certificates pane', () => {
  test('shows expiry countdown, flags failed, and diagnoses a stuck challenge', async ({
    page,
  }, info) => {
    await open(page)
    await openCertificates(page, info)
    await emit(
      page,
      batch([
        system(1, {
          type: 'CertManagerOverviewUpdated',
          data: {
            installed: true,
            certificates: [
              {
                ...base,
                name: 'soon-tls',
                not_after: '2030-01-01T00:00:00Z',
                expiry_state: 'ExpiringSoon',
                expiry_seconds: 5 * 86400,
                ready: 'Ready',
              },
              {
                ...base,
                name: 'broken-tls',
                not_after: null,
                expiry_state: 'Unknown',
                expiry_seconds: null,
                ready: 'Failed',
                message: 'Issuing failed',
              },
            ],
          },
        }),
      ]),
    )

    const soon = page.getByTestId('cert-row').filter({ hasText: 'soon-tls' })
    await expect(soon).toContainText('expires in 5d')
    const broken = page.getByTestId('cert-row').filter({ hasText: 'broken-tls' })
    await expect(broken).toHaveAttribute('data-ready', 'Failed')

    await clearSent(page)
    await broken.click()
    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toContain('SelectCertificateChain')

    await emit(
      page,
      batch([
        system(2, {
          type: 'CertificateChainUpdated',
          data: {
            certificate_name: 'broken-tls',
            certificate_namespace: 'default',
            not_found: false,
            stages: [
              stage('Certificate', 'broken-tls', 'False', false, ''),
              stage('CertificateRequest', 'broken-tls-1', 'False', false, ''),
              stage('Order', 'broken-tls-1-abc', 'pending', false, ''),
              stage(
                'Challenge',
                'broken-tls-1-abc-0',
                'pending',
                true,
                'HTTP-01 propagation failed',
              ),
            ],
          },
        }),
      ]),
    )

    await expect(page.getByTestId('cert-chain-stage')).toHaveCount(4)
    const stuck = page.locator('[data-testid="cert-chain-stage"][data-stuck="true"]')
    await expect(stuck).toHaveCount(1)
    await expect(stuck).toHaveAttribute('data-stage', 'Challenge')
    await expect(stuck).toContainText('HTTP-01 propagation failed')
  })

  test('renders the honest not-installed empty state', async ({ page }, info) => {
    await open(page)
    await openCertificates(page, info)
    await emit(
      page,
      batch([
        system(1, {
          type: 'CertManagerOverviewUpdated',
          data: { installed: false, certificates: [] },
        }),
      ]),
    )
    await expect(page.getByText('cert-manager is not installed in this cluster.')).toBeVisible()
    await expect(page.getByText(/no Certificates were found/)).toHaveCount(0)
  })
})

function stage(kind: string, name: string, state: string, is_stuck: boolean, message: string) {
  return {
    kind: kind as 'Certificate',
    name,
    namespace: 'default',
    state,
    reason: '',
    message,
    is_stuck,
  }
}
