/**
 * LIVE writes (US3) in the fathom-dev sandbox, through the real backend and the devbridge's
 * write guard. Every spec checks the UI AND kubectl, and `restoreBaseline()` runs after each one
 * (scale-me back to 1, every `fathom.dev/e2e=true` object deleted).
 *
 * Targets are sandbox workloads meant for this (scale-me, restart-me, web) or throwaway pods the
 * spec creates. The guard spec never names a real object outside the sandbox: its UI write is
 * rewritten on the wire to a pod name that does not exist, so even a broken guard deletes nothing.
 */
import type { Page } from '@playwright/test'

import type { BackendCmd } from '../../src/protocol/types'

import {
  actionToast,
  checkRows,
  confirmDialog,
  createThrowawayPods,
  DEPLOYMENTS,
  expect,
  findRow,
  getDeployment,
  inspector,
  key,
  LIVE_NAMESPACE,
  listPods,
  PODS,
  podNames,
  renderedKeys,
  restoreBaseline,
  row,
  scaleDialog,
  search,
  showList,
  table,
  tapBridge,
  test,
  waitFor,
  waitForPodsGone,
  waitForPodsReady,
  type BridgeTap,
  type LiveApp,
} from './helpers'

/** A pod name no cluster has: what a write outside the sandbox is rewritten to. */
const PROBE = 'fathom-e2e-guard-probe-does-not-exist'
const FORBIDDEN_TOAST = /Refused by the dev bridge write guard \(Forbidden\)/

test.describe.configure({ timeout: 240_000 })

/** Tap the bridge socket, load the app, and show the sandbox namespace. */
async function start(
  page: Page,
  app: LiveApp,
  rewrite?: (cmd: BackendCmd) => BackendCmd,
): Promise<BridgeTap> {
  const tap = await tapBridge(page, rewrite)
  await app.open()
  await app.selectContext()
  await app.selectNamespace(LIVE_NAMESPACE)
  return tap
}

function successToast(page: Page, text: string | RegExp) {
  return actionToast(page, 'success').filter({ hasText: text })
}

/** Ready replicas of `name` as the UI shows them (`ready/desired`), list column or inspector. */
async function expectUiReady(page: Page, name: string, ready: number, desired: number) {
  const r = await findRow(page, name, DEPLOYMENTS)
  const cell = r.locator('[data-col="ready"]')
  if ((await cell.count()) > 0) {
    await expect(cell).toHaveText(`${String(ready)}/${String(desired)}`, { timeout: 60_000 })
    return
  }
  await r.click()
  const insp = inspector(page)
  await expect(insp).toContainText(new RegExp(`Desired\\s*${String(desired)}`), { timeout: 60_000 })
  await expect(insp).toContainText(new RegExp(`Ready\\s*${String(ready)}`), { timeout: 60_000 })
  // Back to the list (phone: the inspector covers it).
  await showList(page, DEPLOYMENTS, 'Deployments')
}

/** kubectl: `name` has `n` desired and `n` ready replicas. */
function waitForReplicas(name: string, n: number) {
  return waitFor(
    () => {
      const d = getDeployment(name)
      return (
        d.spec?.replicas === n &&
        (d.status?.readyReplicas ?? 0) === n &&
        (d.status?.replicas ?? 0) === n
      )
    },
    {
      timeout: 120_000,
      interval: 2_000,
      message: `deployment ${name} at ${String(n)}/${String(n)}`,
    },
  )
}

async function scaleInUi(page: Page, name: string, from: number, to: number) {
  await findRow(page, name, DEPLOYMENTS)
  await checkRows(page, [name], { section: DEPLOYMENTS })
  await page.getByTestId('bulk-scale').click()
  const dialog = scaleDialog(page)
  await expect(dialog).toBeVisible()
  await expect(dialog.getByTestId('scale-target')).toContainText(key(name))
  await expect(dialog.getByTestId('scale-current')).toHaveText(String(from))
  await dialog.getByLabel('Replicas', { exact: true }).fill(String(to))
  await expect(dialog.getByTestId('scale-new')).toHaveText(String(to))
  await dialog.getByTestId('scale-submit').click()
  await expect(dialog).toBeHidden()
  await expect(successToast(page, `Scaled deployment ${key(name)} to ${String(to)}`)).toBeVisible({
    timeout: 60_000,
  })
}

/**
 * Point every write outside the sandbox at `PROBE` (a pod that does not exist). The command
 * still reaches the guard with its real namespace, so the refusal is real; the name is harmless.
 */
function defuse(cmd: BackendCmd): BackendCmd {
  if (cmd.type === 'BulkDelete') {
    return {
      type: 'BulkDelete',
      data: {
        items: cmd.data.items.map(([ns, name]) => [ns, ns === LIVE_NAMESPACE ? name : PROBE]),
      },
    }
  }
  const data = (cmd as { data?: Record<string, unknown> }).data
  if (!/^(Delete|Scale|Restart|Rollback|Cordon|Uncordon|Drain|Evict)/.test(cmd.type)) return cmd
  if (data === undefined || data.namespace === LIVE_NAMESPACE) return cmd
  return { ...cmd, data: { ...data, name: PROBE } } as BackendCmd
}

test.describe('live: write actions in fathom-dev', () => {
  test.afterEach(() => {
    restoreBaseline()
  })

  test('scale scale-me 1 → 2 → 1', async ({ page, app }) => {
    await waitForReplicas('scale-me', 1)
    const tap = await start(page, app)
    await showList(page, DEPLOYMENTS, 'Deployments')
    await expectUiReady(page, 'scale-me', 1, 1)

    await scaleInUi(page, 'scale-me', 1, 2)
    expect(tap.writes()).toContainEqual({
      type: 'ScaleDeployment',
      data: { namespace: LIVE_NAMESPACE, name: 'scale-me', replicas: 2 },
    })
    await waitForReplicas('scale-me', 2)
    await waitFor(
      () => {
        const pods = listPods('app=scale-me').filter((p) => !p.metadata.deletionTimestamp)
        return (
          pods.length === 2 &&
          pods.every((p) => (p.status?.containerStatuses ?? []).every((c) => c.ready))
        )
      },
      { timeout: 120_000, message: '2 ready scale-me pods' },
    )
    await expectUiReady(page, 'scale-me', 2, 2)

    await scaleInUi(page, 'scale-me', 2, 1)
    await waitForReplicas('scale-me', 1)
    await expectUiReady(page, 'scale-me', 1, 1)
  })

  test('restart restart-me replaces its pod', async ({ page, app }) => {
    const before = await waitFor(
      () => {
        const names = podNames('app=restart-me')
        return names.length > 0 ? names : undefined
      },
      { message: 'a restart-me pod' },
    )
    await waitForPodsReady(before)
    const tap = await start(page, app)
    await showList(page, DEPLOYMENTS, 'Deployments')
    await findRow(page, 'restart-me', DEPLOYMENTS)
    await checkRows(page, ['restart-me'], { section: DEPLOYMENTS })
    await page.getByTestId('bulk-restart').click()

    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('confirm-item')).toHaveCount(1)
    await expect(dialog.getByTestId('confirm-item')).toContainText(key('restart-me'))
    await dialog.getByTestId('confirm-submit').click()
    await expect(successToast(page, `Restarted deployment ${key('restart-me')}`)).toBeVisible({
      timeout: 60_000,
    })
    expect(tap.writes()).toEqual([
      { type: 'RestartDeployment', data: { namespace: LIVE_NAMESPACE, name: 'restart-me' } },
    ])

    // kubectl: a new pod, ready, and the old one gone.
    const after = await waitFor(
      () => {
        const names = podNames('app=restart-me').filter((n) => !before.includes(n))
        return names.length > 0 ? names : undefined
      },
      { timeout: 120_000, message: 'a new restart-me pod' },
    )
    await waitForPodsReady(after)
    await waitForPodsGone(before)

    // UI: the same.
    await showList(page, PODS, 'Pods')
    for (const name of after) await findRow(page, name)
    for (const name of before) {
      await search(page, name)
      await expect(row(page, key(name))).toHaveCount(0, { timeout: 60_000 })
    }
  })

  test('delete one web pod: success toast, and the ReplicaSet replaces it', async ({
    page,
    app,
  }) => {
    const web = podNames('app=web')
    const victim = web[0]
    if (victim === undefined) throw new Error('no web pod: reset the sandbox')
    await waitForPodsReady(web)
    const tap = await start(page, app)

    await findRow(page, victim)
    await checkRows(page, [victim])
    await page.getByTestId('bulk-delete').click()
    const dialog = confirmDialog(page)
    await expect(dialog.getByTestId('confirm-item')).toHaveCount(1)
    await expect(dialog.getByTestId('confirm-item')).toContainText(key(victim))
    await dialog.getByTestId('confirm-submit').click()
    await expect(successToast(page, `Deleted pod ${key(victim)}`)).toBeVisible({
      timeout: 60_000,
    })
    expect(tap.writes()).toEqual([
      { type: 'DeletePod', data: { namespace: LIVE_NAMESPACE, name: victim } },
    ])

    await waitForPodsGone([victim])
    const replacements = await waitFor(
      () => {
        const names = podNames('app=web').filter((n) => !web.includes(n))
        return names.length > 0 ? names : undefined
      },
      { timeout: 120_000, message: 'a replacement web pod' },
    )
    await waitForPodsReady(replacements)
    expect(podNames('app=web')).toHaveLength(web.length)

    await search(page, victim)
    await expect(row(page, key(victim))).toHaveCount(0, { timeout: 60_000 })
    for (const name of replacements) await findRow(page, name)
  })

  test('bulk-delete 3 throwaway pods', async ({ page, app }) => {
    const names = createThrowawayPods(3)
    const prefix = names[0]?.replace(/-1$/, '') ?? ''
    await waitForPodsReady(names)
    const tap = await start(page, app)

    await search(page, prefix)
    await expect(table(page)).toHaveAttribute('data-row-count', '3', { timeout: 60_000 })
    await checkRows(page, names, { query: prefix })
    await expect(page.getByTestId('selection-count')).toContainText('3 selected')
    await page.getByTestId('bulk-delete').click()

    const dialog = confirmDialog(page)
    await expect(dialog.getByTestId('confirm-item')).toHaveCount(3)
    for (const name of names) await expect(dialog).toContainText(key(name))
    await expect(dialog.getByTestId('confirm-typed-count')).toHaveCount(0)
    await dialog.getByTestId('confirm-submit').click()
    await expect(successToast(page, 'Deleted 3 pods')).toBeVisible({ timeout: 60_000 })

    const bulk = tap.writes()
    expect(bulk).toHaveLength(1)
    const [cmd] = bulk
    expect(cmd?.type).toBe('BulkDelete')
    if (cmd?.type === 'BulkDelete') {
      expect([...cmd.data.items].sort()).toEqual(names.map((n) => [LIVE_NAMESPACE, n]).sort())
    }

    await waitForPodsGone(names)
    await search(page, prefix)
    await expect(table(page)).toHaveAttribute('data-row-count', '0', { timeout: 60_000 })
  })

  test('Cancel sends nothing', async ({ page, app }) => {
    const [name] = createThrowawayPods(1, 'e2e-cancel')
    if (name === undefined) throw new Error('unreachable')
    const tap = await start(page, app)

    await findRow(page, name)
    await checkRows(page, [name])
    await page.getByTestId('bulk-delete').click()
    const dialog = confirmDialog(page)
    await expect(dialog.getByTestId('confirm-item')).toContainText(key(name))
    await dialog.getByTestId('confirm-cancel').click()
    await expect(dialog).toBeHidden()

    // Give a stray send time to show up, then check the wire and the cluster.
    await page.waitForTimeout(3_000)
    expect(tap.writes()).toEqual([])
    await expect(actionToast(page, 'loading')).toHaveCount(0)
    await expect(actionToast(page, 'success')).toHaveCount(0)
    expect(podNames('app=e2e-throwaway')).toContain(name)
    await expect(row(page, key(name))).toBeVisible()
  })

  test('a write outside fathom-dev is refused (Forbidden) and changes nothing', async ({
    page,
    app,
  }) => {
    const tap = await start(page, app, defuse)
    const baseline = podNames()

    // 1. Raw: the bridge itself refuses writes outside the sandbox (names that don't exist).
    const raw = await page.evaluate(async (probe) => {
      const bridge = window.__fathomDevBridge
      if (!bridge) return ['no __fathomDevBridge on the page']
      const attempts: BackendCmd[] = [
        { type: 'DeletePod', data: { namespace: 'default', name: probe } },
        { type: 'BulkDelete', data: { items: [['default', probe]] } },
        { type: 'ScaleDeployment', data: { namespace: 'default', name: probe, replicas: 0 } },
      ]
      const out: string[] = []
      for (const cmd of attempts) {
        try {
          await bridge.send(cmd)
          out.push(`${cmd.type}: ACCEPTED`)
        } catch (err) {
          const e = err as { type?: string; data?: string }
          out.push(`${cmd.type}: ${e.type ?? '?'}: ${e.data ?? ''}`)
        }
      }
      return out
    }, PROBE)
    expect(raw).toHaveLength(3)
    for (const line of raw) expect(line).toMatch(/: Forbidden: .*fathom-dev/)

    // 2. Through the UI: all namespaces, a pod outside the sandbox, Delete. The command reaches
    //    the guard with its real namespace but a name rewritten to PROBE (see `defuse`).
    await app.selectNamespace('all')
    await search(page, '')
    const outside = await waitFor(
      async () =>
        (await renderedKeys(page)).find((k) => k !== '' && !k.startsWith(`${LIVE_NAMESPACE}/`)),
      { timeout: 60_000, message: 'a pod outside fathom-dev in the all-namespaces list' },
    )
    await search(page, outside)
    const target = row(page, outside)
    await expect(target).toBeVisible()
    const box = target.getByRole('checkbox', { name: `Select ${outside}` })
    await box.click()
    await expect(box).toHaveAttribute('aria-checked', 'true')
    await page.getByTestId('bulk-delete').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()

    const toast = actionToast(page, 'error').filter({ hasText: FORBIDDEN_TOAST })
    await expect(toast).toBeVisible({ timeout: 30_000 })
    await expect(toast).toContainText(`outside the sandbox namespace '${LIVE_NAMESPACE}'`)

    const [ns] = outside.split('/')
    expect(tap.writes()).toContainEqual({ type: 'DeletePod', data: { namespace: ns, name: PROBE } })
    expect(tap.errors.filter((e) => e.type === 'Forbidden').length).toBeGreaterThanOrEqual(4)

    // Still connected, the row is still listed, and nothing in the sandbox moved.
    await expect(page.locator('[data-pane-state="disconnected"]')).toHaveCount(0)
    await expect(target).toBeVisible()
    const now = podNames()
    for (const name of baseline) expect(now).toContain(name)

    // Leave the filter on the sandbox for later specs (the backend remembers it).
    await app.selectNamespace(LIVE_NAMESPACE)
  })
})
