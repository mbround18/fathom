/**
 * Promo scenes, captured live from the `fathom-dev` sandbox (read-only; see helpers.ts). Each
 * scene writes one PNG to promo-out/shots/; `compose.spec.ts` runs after and builds the PDF.
 *
 * Dialogs (scale, bulk delete) are opened and photographed, then dismissed: nothing is confirmed.
 */
import type { Page } from '@playwright/test'

import { check } from '../e2e/helpers'
import { DEPLOYMENTS, inspector, key, navigateTo, row, showList, table } from '../live/helpers'

import { expect, LIVE_NAMESPACE, listPods, redactions, shoot, test } from './helpers'

/** The sandbox's single synthesized Application: `dev/sandbox/kustomization.yaml` stamps
 * `app.kubernetes.io/part-of: fathom-sandbox` onto every resource in the namespace, which the
 * backend's label-heuristic attribution tier (`src/backend/applications.rs`) turns into one
 * Application — instance/name `fathom-sandbox`, owning every sandbox workload. */
const APPLICATIONS = 'applications'
const APP_INSTANCE = 'fathom-sandbox'

/** The sandbox's one Flux-managed Helm release (`dev/sandbox/helm.yaml`): a real podinfo
 * install, upgraded once, so both the release list/inspector and the origin-link click-through
 * to its owning Flux `HelmRelease` have real, live data to show. */
const HELM = 'helm/releases'
const HELM_RELEASE = 'podinfo'

/** A live pod of `app=<app>` (the first by name, so reruns pick the same one). */
function podOf(app: string): string {
  const names = listPods(`app=${app}`)
    .filter((p) => !p.metadata.deletionTimestamp)
    .map((p) => p.metadata.name)
    .sort()
  const name = names[0]
  if (!name) {
    throw new Error(
      `no live pod with app=${app} in ${LIVE_NAMESPACE}; ` +
        `run 'FATHOM_SANDBOX_CONTEXT=<ctx> scripts/sandbox.sh reset'`,
    )
  }
  return name
}

async function ready(page: Page, app: { open(): Promise<void>; selectContext(): Promise<void>; selectNamespace(n: string): Promise<void> }): Promise<void> {
  await app.open()
  await app.selectContext()
  await app.selectNamespace(LIVE_NAMESPACE)
  // Every sandbox workload is listed before anything is photographed.
  await expect
    .poll(async () => Number(await table(page).getAttribute('data-row-count')), {
      timeout: 30_000,
    })
    .toBeGreaterThanOrEqual(listPods().length)
}

async function openInspector(page: Page, podName: string): Promise<void> {
  await row(page, key(podName)).click()
  await expect(inspector(page).getByRole('heading', { name: podName })).toBeVisible()
}

test.describe('wide', () => {
  test.use({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 2 })

  test('overview with inspector', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await openInspector(page, podOf('web'))
    await shoot(page, 'wide-inspector', pairs)
  })

  test('scale dialog', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await showList(page, DEPLOYMENTS, 'Deployments')
    await row(page, key('scale-me'), DEPLOYMENTS).click({ button: 'right' })
    await page.getByTestId('row-action-menu').getByTestId('action-scale').click()
    const dialog = page.getByTestId('scale-dialog')
    await dialog.getByLabel('Replicas', { exact: true }).fill('3')
    await expect(dialog.getByTestId('scale-submit')).toHaveText('Scale 1 → 3')
    await shoot(page, 'wide-scale', pairs)
    await dialog.getByTestId('scale-cancel').click()
  })

  test('bulk delete confirmation', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    for (const a of ['crashloop', 'bad-image', 'unschedulable']) await check(page, key(podOf(a)))
    await expect(page.getByTestId('selection-count')).toContainText('3 selected')
    await page.getByTestId('bulk-delete').click()
    const dialog = page.getByTestId('confirm-dialog')
    await expect(dialog.getByTestId('confirm-item')).toHaveCount(3)
    await shoot(page, 'wide-confirm', pairs)
    await dialog.getByTestId('confirm-cancel').click()
  })

  test('flux panel', async ({ app, page, toolPane }) => {
    const pairs = redactions()
    await ready(page, app)
    await toolPane.open(page, 'flux')
    await shoot(page, 'wide-flux', pairs)
  })

  test('hubble panel', async ({ app, page, toolPane }) => {
    const pairs = redactions()
    await ready(page, app)
    await toolPane.open(page, 'hubble')
    await shoot(page, 'wide-hubble', pairs)
  })

  test('logs tab', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    // chatty-logger emits ~5 lines/sec, so real streamed output is available almost immediately.
    await openInspector(page, podOf('chatty-logger'))
    const insp = inspector(page)
    await insp.getByRole('tab', { name: 'Logs' }).click()
    await expect(insp.getByRole('tab', { name: 'Logs' })).toHaveAttribute('aria-selected', 'true')
    const scroll = insp.getByTestId('pod-logs-scroll')
    // Wait for real streamed lines (not just the "starting" placeholder) before shooting.
    await expect
      .poll(async () => (await scroll.innerText()).split('\n').filter((l) => l.trim()).length, {
        timeout: 15_000,
      })
      .toBeGreaterThan(0)
    await shoot(page, 'wide-logs', pairs)
  })

  test('applications list', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await navigateTo(page, 'Applications', 'Overview')
    await expect
      .poll(async () => Number(await table(page, APPLICATIONS).getAttribute('data-row-count')), {
        timeout: 30_000,
      })
      .toBeGreaterThanOrEqual(1)
    await shoot(page, 'wide-applications-list', pairs)
  })

  test('applications inspector', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await navigateTo(page, 'Applications', 'Overview')
    await row(page, key(APP_INSTANCE), APPLICATIONS).click()
    const insp = inspector(page)
    await expect(insp.getByRole('heading', { name: APP_INSTANCE })).toBeVisible()
    // Related resources stream in after the initial select; wait for at least one link (the
    // sandbox's `web` Deployment, owned via the label heuristic) before photographing.
    await expect
      .poll(async () => insp.getByRole('button').filter({ hasText: 'owns' }).count())
      .toBeGreaterThan(0)
    await shoot(page, 'wide-applications-inspector', pairs)
  })

  test('applications drilldown', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await navigateTo(page, 'Applications', 'Overview')
    await row(page, key(APP_INSTANCE), APPLICATIONS).click()
    const insp = inspector(page)
    await expect(insp.getByRole('heading', { name: APP_INSTANCE })).toBeVisible()
    const podLink = insp.getByRole('button').filter({ hasText: 'owns' }).filter({ hasText: 'Pod' }).first()
    await expect(podLink).toBeVisible()
    const podName = (await podLink.locator('span').first().innerText()).split('/').pop()
    await podLink.click()
    const breadcrumb = page.getByRole('navigation', { name: 'Breadcrumb' })
    await expect(breadcrumb).toBeVisible()
    await expect(breadcrumb).toContainText(APP_INSTANCE)
    if (podName) await expect(inspector(page).getByRole('heading', { name: podName })).toBeVisible()
    await shoot(page, 'wide-applications-drilldown', pairs)
  })

  test('helm releases list', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await navigateTo(page, 'Helm Releases', 'Overview')
    await expect
      .poll(async () => Number(await table(page, HELM).getAttribute('data-row-count')), {
        timeout: 30_000,
      })
      .toBeGreaterThanOrEqual(1)
    await shoot(page, 'wide-helm-list', pairs)
  })

  test('helm release inspector', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await navigateTo(page, 'Helm Releases', 'Overview')
    await row(page, key(HELM_RELEASE), HELM).click()
    const insp = inspector(page)
    await expect(insp.getByRole('heading', { name: HELM_RELEASE })).toBeVisible()
    // Revision history streams in after the initial select; wait for both real revisions
    // (v1 install, v2 upgrade) before photographing.
    await expect
      .poll(async () => insp.getByText('#2', { exact: true }).count())
      .toBeGreaterThan(0)
    await shoot(page, 'wide-helm-inspector', pairs)
  })

  test('helm origin link', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await navigateTo(page, 'Helm Releases', 'Overview')
    await row(page, key(HELM_RELEASE), HELM).click()
    const insp = inspector(page)
    await expect(insp.getByRole('heading', { name: HELM_RELEASE })).toBeVisible()
    const originLink = insp.getByRole('button', { name: `fathom-dev/${HELM_RELEASE}`, exact: true })
    await expect(originLink).toBeVisible()
    await originLink.click()
    const breadcrumb = page.getByRole('navigation', { name: 'Breadcrumb' })
    await expect(breadcrumb).toBeVisible()
    await expect(breadcrumb).toContainText(HELM_RELEASE)
    const fluxInsp = inspector(page)
    await expect(fluxInsp.getByRole('heading', { name: HELM_RELEASE })).toBeVisible()
    // The Flux resource's own details (Ready state, message, revision) stream in after the
    // click-through; wait for the real reconciliation message before photographing, not the
    // loading skeleton.
    await expect(fluxInsp.getByText(/succeeded/)).toBeVisible()
    await shoot(page, 'wide-helm-origin-link', pairs)
  })

  test('topology graph view', async ({ app, page, toolPane }) => {
    const pairs = redactions()
    await ready(page, app)
    // The canvas is hand-drawn (data-model.md: thousands of pods must stay responsive), so
    // "real data rendered" isn't a DOM assertion — sample its own pixels for drawn (non-
    // transparent) content instead of trusting a fixed settle time.
    await toolPane.open(page, 'topology', async (pg) => {
      const canvas = pg.getByTestId('topology-canvas')
      await expect(canvas).toBeVisible()
      await expect
        .poll(
          () =>
            pg.evaluate(() => {
              const el = document.querySelector<HTMLCanvasElement>(
                '[data-testid="topology-canvas"]',
              )
              const ctx = el?.getContext('2d')
              if (!el || !ctx) return 0
              const { data } = ctx.getImageData(0, 0, el.width, el.height)
              let drawn = 0
              for (let i = 3; i < data.length; i += 400) if ((data[i] ?? 0) > 0) drawn++
              return drawn
            }),
          { timeout: 20_000, message: 'topology graph nodes to render' },
        )
        .toBeGreaterThan(0)
    })
    await shoot(page, 'wide-topology', pairs)
  })

  test('pod lifecycle diagnostics', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    // `crashloop` has real restart history and a failing probe (dev/sandbox/workloads.yaml), so
    // the diagnostics sections have genuine content instead of an all-empty panel.
    await openInspector(page, podOf('crashloop'))
    const insp = inspector(page)
    const lifecycle = insp.getByTestId('pod-lifecycle-restarts')
    await lifecycle.scrollIntoViewIfNeeded()
    await expect(lifecycle).toBeVisible()
    await expect(insp.getByTestId('pod-lifecycle-events-timeline')).toBeVisible()
    await shoot(page, 'wide-pod-lifecycle', pairs)
  })

  test('port-forward start dialog', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    // `StartPortForward` is not on helpers.ts's blocked-write list (it isn't a cluster mutation
    // in the same sense as Delete/Scale/…), but actually starting one here would leave a real
    // tunnel process running past the end of this read-only scene with nothing to tear it down.
    // So: open the real dialog, with the real target port filled in, and dismiss without
    // confirming — the same "open, photograph, cancel" pattern as the scale/bulk-delete scenes.
    await openInspector(page, podOf('web'))
    const insp = inspector(page)
    await insp.getByRole('tab', { name: 'Forward' }).click()
    await insp.getByTestId(/^port-forward-action-/).first().click()
    const dialog = page.getByTestId('confirm-dialog')
    const detail = page.getByTestId('port-forward-start-dialog')
    await expect(detail).toBeVisible()
    await expect(detail.getByLabel('Local port')).not.toHaveValue('')
    await shoot(page, 'wide-port-forward', pairs)
    await dialog.getByTestId('confirm-cancel').click()
  })

  test('container image inspection', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await openInspector(page, podOf('web'))
    const insp = inspector(page)
    await insp.getByTestId('image-inspect-tab').click()
    const panel = insp.getByTestId('image-inspect-panel')
    await expect(panel).toBeVisible()
    // A real registry pull/inspect can take a while; wait for either the real result or its
    // honest error state, never the loading skeleton.
    await expect(panel.getByTestId('image-inspect-ref').or(panel.getByTestId(/^image-inspect-error-/))).toBeVisible({
      timeout: 30_000,
    })
    await shoot(page, 'wide-image-inspect', pairs)
  })

  test('rbac role bindings list', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await navigateTo(page, 'Role Bindings', 'Access Control')
    await expect
      .poll(async () => Number(await table(page, 'access/rolebindings').getAttribute('data-row-count')), {
        timeout: 30_000,
      })
      .toBeGreaterThanOrEqual(1)
    await shoot(page, 'wide-rbac-rolebindings', pairs)
  })

  test('federation context switcher', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await openInspector(page, podOf('web'))
    const switcher = page.getByTestId('federation-context-switcher')
    await expect(switcher).toBeVisible()
    await expect(page.getByTestId('federation-context-item').filter({ hasText: app.context })).toHaveAttribute(
      'data-context-status',
      'connected',
    )
    await shoot(page, 'wide-federation-context', pairs)
  })

  test('agent ops panel', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await openInspector(page, podOf('web'))
    const insp = inspector(page)
    await insp.getByTestId('agent-ops-tab').click()
    // No LLM/Ollama is configured in this sandbox, so there is no real conversation to show —
    // the honest state here is the panel's own empty prompt, not a fabricated exchange.
    await expect(insp.getByTestId('agent-ops-empty')).toBeVisible()
    await shoot(page, 'wide-agent-ops', pairs)
  })

  test('cluster settings panel', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await navigateTo(page, 'Cluster Settings', 'Team')
    const panel = page.getByTestId('teams-cluster-settings-panel')
    await expect(panel).toBeVisible()
    await shoot(page, 'wide-cluster-settings', pairs)
  })

  test('team connection profiles panel', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await navigateTo(page, 'Connection Profiles', 'Team')
    const panel = page.getByTestId('teams-profiles-panel')
    await expect(panel).toBeVisible()
    await shoot(page, 'wide-team-profiles', pairs)
  })
})

test.describe('ultra', () => {
  test.use({ viewport: { width: 3840, height: 1600 }, deviceScaleFactor: 1 })

  test('six-slot overview', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await openInspector(page, podOf('web'))
    await shoot(page, 'ultra', pairs)
  })
})

test.describe('tablet', () => {
  test.use({ viewport: { width: 820, height: 1180 }, deviceScaleFactor: 2, hasTouch: true })

  test('list', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await shoot(page, 'tablet', pairs)
  })
})

test.describe('phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, hasTouch: true })

  test('list', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await shoot(page, 'phone-list', pairs)
  })

  test('inspector', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await openInspector(page, podOf('web'))
    await shoot(page, 'phone-inspector', pairs)
  })

  test('navigator', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await page.getByRole('button', { name: 'Open navigator' }).click()
    await expect(page.getByRole('navigation', { name: 'Sections' })).toBeVisible()
    await shoot(page, 'phone-nav', pairs)
  })
})
