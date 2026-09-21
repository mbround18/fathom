/**
 * LIVE, read-only: browse the fathom-dev sandbox through the real backend and compare what the
 * UI shows with `kubectl`. Nothing here writes to the cluster.
 *
 * Status note: the UI's pod Status column shows `.status.phase` today (PodSummary carries only
 * the phase), while `kubectl get pods` shows the container reason (CrashLoopBackOff,
 * ErrImagePull, …). The checks accept either; the fixme at the end is the strict version.
 */
import type { Page } from '@playwright/test'

import {
  DEPLOYMENTS,
  expect,
  findRow,
  getDeployment,
  inspector,
  key,
  kubectlStatus,
  listPods,
  LIVE_NAMESPACE,
  podNames,
  podPhase,
  renderedKeys,
  rowCount,
  search,
  showList,
  statusCell,
  table,
  test,
  waitFor,
  type KubePod,
} from './helpers'

const RESET_HINT = `run 'FATHOM_SANDBOX_CONTEXT=<ctx> scripts/sandbox.sh reset'`

/** The one pod of a single-replica sandbox deployment, waiting until kubectl reports it. */
function podOf(app: string): Promise<KubePod> {
  return waitFor(
    () => {
      const live = listPods(`app=${app}`).filter((p) => !p.metadata.deletionTimestamp)
      return live.length === 1 ? live[0] : undefined
    },
    { timeout: 30_000, message: `exactly one live pod with app=${app} (${RESET_HINT})` },
  )
}

/** Wait until kubectl's STATUS for the pod labelled `app=<app>` matches `re`. */
function podWithStatus(app: string, re: RegExp): Promise<KubePod> {
  return waitFor(
    async () => {
      const pod = await podOf(app)
      return re.test(kubectlStatus(pod)) ? pod : undefined
    },
    { timeout: 90_000, interval: 2_000, message: `app=${app} to show ${String(re)} in kubectl` },
  )
}

/** The UI Status for `name` is the phase (today) or kubectl's reason (once the UI shows it). */
async function expectUiStatus(page: Page, name: string, pod: KubePod): Promise<void> {
  const r = await findRow(page, name)
  const accepted = [podPhase(pod), kubectlStatus(pod)].map((s) =>
    s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'),
  )
  await expect(statusCell(r)).toHaveText(new RegExp(`(${accepted.join('|')})`))
}

test.describe('live: browse fathom-dev (read-only)', () => {
  test.beforeEach(async ({ app }) => {
    await app.open()
    await app.selectContext()
    await app.selectNamespace(LIVE_NAMESPACE)
  })

  test('the pods list matches kubectl, including the broken sandbox workloads', async ({
    page,
  }) => {
    // Same set size as kubectl (re-read each poll: the cronjob can add a pod at :00 / :30).
    await expect
      .poll(async () => (await rowCount(page)) - listPods().length, {
        message: 'UI pod count minus kubectl pod count',
        timeout: 30_000,
      })
      .toBe(0)

    // Healthy workloads: every kubectl pod has a row with the same status.
    for (const app of ['web', 'chatty-logger']) {
      const pods = listPods(`app=${app}`).filter((p) => !p.metadata.deletionTimestamp)
      expect(pods.length, `kubectl pods for app=${app} (${RESET_HINT})`).toBeGreaterThan(0)
      if (app === 'web') expect(pods).toHaveLength(2)
      for (const pod of pods) {
        expect(podPhase(pod)).toBe('Running')
        await expectUiStatus(page, pod.metadata.name, pod)
      }
    }

    // crashloop: kubectl says CrashLoopBackOff (or Error between restarts).
    const crash = await podWithStatus('crashloop', /^(CrashLoopBackOff|Error)$/)
    await expectUiStatus(page, crash.metadata.name, crash)

    // unschedulable: Pending everywhere.
    const pending = await podOf('unschedulable')
    expect(kubectlStatus(pending)).toBe('Pending')
    await expect(statusCell(await findRow(page, pending.metadata.name))).toContainText('Pending')

    // bad-image: kubectl says ImagePullBackOff / ErrImagePull.
    const bad = await podWithStatus('bad-image', /^(ImagePullBackOff|ErrImagePull)$/)
    await expectUiStatus(page, bad.metadata.name, bad)
  })

  test('the deployments list shows scale-me at 1/1', async ({ page }) => {
    const d = getDeployment('scale-me')
    expect(
      [d.spec?.replicas, d.status?.readyReplicas ?? 0],
      `kubectl: scale-me should be 1/1 at baseline (${RESET_HINT})`,
    ).toEqual([1, 1])

    await showList(page, DEPLOYMENTS, 'Deployments')
    const r = await findRow(page, 'scale-me', DEPLOYMENTS)
    const ready = r.locator('[data-col="ready"]')
    if ((await ready.count()) > 0) {
      await expect(ready).toHaveText('1/1')
    } else {
      // Too narrow for the Ready column: the inspector's Status section has the numbers.
      await r.click()
      const insp = inspector(page)
      await expect(insp.getByRole('heading', { name: 'scale-me' })).toBeVisible()
      await expect(insp).toContainText(/Desired\s*1/)
      await expect(insp).toContainText(/Ready\s*1/)
    }
  })

  test('the inspector shows a web pod: Details, YAML (namespace fathom-dev), Events', async ({
    page,
  }) => {
    const [name] = podNames('app=web')
    if (!name) throw new Error(`no web pod in kubectl (${RESET_HINT})`)

    await (await findRow(page, name)).click()
    const insp = inspector(page)
    await expect(insp.getByRole('heading', { name })).toBeVisible()

    const details = insp.getByRole('tab', { name: 'Details' })
    await expect(details).toHaveAttribute('aria-selected', 'true')
    for (const h of ['Metadata', 'Status', 'Spec', 'Labels']) {
      await expect(insp.getByRole('heading', { name: h, exact: true })).toBeVisible()
    }
    await expect(insp).toContainText('ReplicaSet')
    await expect(insp).toContainText(LIVE_NAMESPACE)

    await insp.getByRole('tab', { name: 'YAML' }).click()
    const yaml = insp.locator('pre')
    await expect(yaml).toContainText('kind: Pod')
    await expect(yaml).toContainText(`name: ${name}`)
    await expect(yaml).toContainText(`namespace: ${LIVE_NAMESPACE}`)

    const events = insp.getByRole('tab', { name: 'Events' })
    await events.click()
    await expect(events).toHaveAttribute('aria-selected', 'true')
    await expect(insp.getByRole('tabpanel')).toBeVisible()
  })

  test('switching the namespace filter away and back does not leak rows', async ({ page, app }) => {
    // Another namespace from the picker (the UI's own list; kubectl stays fenced to fathom-dev).
    await page.getByRole('combobox', { name: 'Namespace' }).click()
    const options = await page.getByRole('option').allInnerTexts()
    await page.keyboard.press('Escape')
    const others = options.map((o) => o.trim()).filter((o) => o !== LIVE_NAMESPACE && o !== 'all')
    const other = others.includes('kube-system') ? 'kube-system' : others[0]
    test.skip(other === undefined, 'the cluster lists no namespace besides fathom-dev')
    if (other === undefined) return

    await app.selectNamespace(other)
    // No fathom-dev row survives, even after late batches had time to land.
    await expect(table(page).locator(`[data-row-key^="${LIVE_NAMESPACE}/"]`)).toHaveCount(0)
    await page.waitForTimeout(2_000)
    await expect(table(page).locator(`[data-row-key^="${LIVE_NAMESPACE}/"]`)).toHaveCount(0)
    if ((await table(page).count()) > 0) {
      await search(page, LIVE_NAMESPACE)
      await expect(table(page)).toHaveAttribute('data-row-count', '0')
      await search(page, '')
    }

    await app.selectNamespace(LIVE_NAMESPACE)
    await expect
      .poll(async () => (await rowCount(page)) - listPods().length, { timeout: 30_000 })
      .toBe(0)
    await expect(table(page).locator(`[data-row-key^="${other}/"]`)).toHaveCount(0)
    await search(page, other)
    await expect(table(page)).toHaveAttribute('data-row-count', '0')
    await search(page, '')
    for (const k of await renderedKeys(page)) expect(k.startsWith(`${LIVE_NAMESPACE}/`)).toBe(true)
    // Spot-check that a known pod is back.
    const [web] = podNames('app=web')
    if (web) await expect(await findRow(page, web)).toHaveAttribute('data-row-key', key(web))
  })

  test.fixme('the pod Status column shows the container reason, like kubectl', async ({ page }) => {
    // Needs PodSummary to carry the waiting/terminated reason (today it has only `phase`, so a
    // crash-looping pod reads "Running" and a bad image "Pending").
    const crash = await podWithStatus('crashloop', /^(CrashLoopBackOff|Error)$/)
    await expect(statusCell(await findRow(page, crash.metadata.name))).toContainText(
      kubectlStatus(crash),
    )
    const bad = await podWithStatus('bad-image', /^(ImagePullBackOff|ErrImagePull)$/)
    await expect(statusCell(await findRow(page, bad.metadata.name))).toContainText(
      /ImagePullBackOff|ErrImagePull/,
    )
  })
})
