/**
 * LIVE quickstart validation for 004-applications-interlinking, Scenario 4 (T058): edge cases.
 *
 * 1-2. A workload with no Helm/Flux/app.kubernetes.io metadata is not an application, but still
 *      shows in Workloads → Deployments.
 * 3-4. The Flux-managed `podinfo` release loses every owned workload out of band (its Deployment
 *      is deleted; helm-controller has drift detection off, so nothing recreates it until the next
 *      forced reconcile). The application must stay listed with a "No workloads" status (FR-016).
 *      The spec restores it with a forced Flux reconcile.
 */
import { execFileSync } from 'node:child_process'

import type { Page } from '@playwright/test'

import { liveContext } from './env'
import {
  E2E_LABEL,
  expect,
  inspector,
  kubectl,
  LIVE_NAMESPACE,
  navigateTo,
  row,
  search,
  statusCell,
  test,
} from './helpers'

const APPS = 'applications'
const DEPLOYMENTS = 'workloads/deployments'
const BARE = 'qs004-unattributed'
const FLUX_APP = 'podinfo'

function k(args: readonly string[]): string {
  return execFileSync('kubectl', ['--context', liveContext(), '-n', LIVE_NAMESPACE, ...args], {
    encoding: 'utf8',
  })
}

async function showApplications(page: Page): Promise<void> {
  await navigateTo(page, 'Applications', 'Overview')
}

test.describe('live 004 quickstart scenario 4', () => {
  test.beforeEach(async ({ app }) => {
    await app.open()
    await app.selectContext()
    await app.selectNamespace(LIVE_NAMESPACE)
  })

  test('4.1-4.2 an unattributable workload is not an application but is still a Deployment', async ({
    page,
  }) => {
    kubectl(['create', '-f', '-'], {
      input: {
        apiVersion: 'apps/v1',
        kind: 'Deployment',
        metadata: {
          name: BARE,
          namespace: LIVE_NAMESPACE,
          labels: { [E2E_LABEL[0]]: E2E_LABEL[1], app: BARE },
        },
        spec: {
          replicas: 1,
          selector: { matchLabels: { app: BARE } },
          template: {
            metadata: { labels: { [E2E_LABEL[0]]: E2E_LABEL[1], app: BARE } },
            spec: {
              terminationGracePeriodSeconds: 1,
              securityContext: {
                runAsNonRoot: true,
                runAsUser: 65534,
                runAsGroup: 65534,
                seccompProfile: { type: 'RuntimeDefault' },
              },
              containers: [
                {
                  name: 'main',
                  image: 'busybox:1.37',
                  command: ['sh', '-c', 'sleep infinity'],
                  resources: {
                    requests: { cpu: '5m', memory: '8Mi' },
                    limits: { cpu: '20m', memory: '16Mi' },
                  },
                  securityContext: {
                    allowPrivilegeEscalation: false,
                    capabilities: { drop: ['ALL'] },
                  },
                },
              ],
            },
          },
        },
      },
    })
    try {
      await navigateTo(page, 'Deployments')
      await search(page, BARE, DEPLOYMENTS)
      await expect(row(page, `${LIVE_NAMESPACE}/${BARE}`, DEPLOYMENTS)).toBeVisible()

      await showApplications(page)
      await search(page, '', APPS) // the search text may carry over from Deployments
      // podinfo is there, so the list has loaded; the bare workload is not.
      await expect(row(page, `${LIVE_NAMESPACE}/${FLUX_APP}`, APPS)).toBeVisible()
      await search(page, BARE, APPS)
      await expect(row(page, `${LIVE_NAMESPACE}/${BARE}`, APPS)).toHaveCount(0)
      await page.waitForTimeout(12_000) // at least one more applications poll
      await expect(row(page, `${LIVE_NAMESPACE}/${BARE}`, APPS)).toHaveCount(0)
    } finally {
      kubectl(['delete', `deployment/${BARE}`, '--ignore-not-found', '--wait=false'])
    }
  })

  test('4.3-4.4 a Flux application with zero remaining workloads stays listed as "No workloads"', async ({
    page,
  }) => {
    await showApplications(page)
    const r = row(page, `${LIVE_NAMESPACE}/${FLUX_APP}`, APPS)
    await expect(r).toBeVisible()
    // The Managed By column is dropped at this list width; the Inspector always shows it.
    await r.click()
    await expect(inspector(page)).toContainText(/Managed by\s*Flux/)
    await expect(statusCell(r)).toHaveText(/Available|Progressing/)

    k(['delete', 'deployment', FLUX_APP, '--wait=true'])
    try {
      // Re-open the list until the backend has re-synthesized without the Deployment.
      await expect
        .poll(
          async () => {
            await navigateTo(page, 'Helm Releases', 'Overview')
            await showApplications(page)
            return (await statusCell(r).textContent()) ?? ''
          },
          { timeout: 60_000, intervals: [3_000] },
        )
        .toMatch(/No workloads$/)
      await r.click()
      await expect(inspector(page)).toContainText(/Managed by\s*Flux/)
    } finally {
      // Restore: a forced reconcile re-runs the Helm upgrade and recreates the Deployment.
      const now = new Date().toISOString()
      k([
        'annotate',
        `helmrelease.helm.toolkit.fluxcd.io/${FLUX_APP}`,
        `reconcile.fluxcd.io/requestedAt=${now}`,
        `reconcile.fluxcd.io/forceAt=${now}`,
        '--overwrite',
      ])
    }
    await expect
      .poll(() => k(['get', 'deployment', FLUX_APP, '-o', 'name', '--ignore-not-found']).trim(), {
        timeout: 120_000,
        intervals: [3_000],
      })
      .toBe(`deployment.apps/${FLUX_APP}`)
  })
})
