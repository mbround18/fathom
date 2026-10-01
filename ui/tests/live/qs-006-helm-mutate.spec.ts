/**
 * LIVE quickstart validation for 006-helm-mutate (T036): rollback, upgrade and uninstall
 * against real Helm releases in the fathom-dev sandbox.
 *
 * Needs (set up outside the spec, see specs/006-helm-mutate/checklists/live-validation.md):
 * - `podinfo`: the Flux-managed release from dev/sandbox/helm.yaml, with >= 2 revisions;
 * - `quickstart-bare`: a bare `helm install` of the same chart in fathom-dev, >= 2 revisions.
 *
 * Every write goes through the devbridge, which only forwards writes into fathom-dev. Flux-owned
 * `podinfo` is never actually mutated: its scenarios stop at the warning and cancel.
 */
import { execFileSync } from 'node:child_process'

import type { Page } from '@playwright/test'

import { liveContext } from './env'
import { confirmDialog, expect, inspector, LIVE_NAMESPACE, navigateTo, row, test } from './helpers'

const HELM = 'helm/releases'
const BARE = 'quickstart-bare'
const FLUX = 'podinfo'

function helm(args: readonly string[]): string {
  return execFileSync('helm', ['--kube-context', liveContext(), '-n', LIVE_NAMESPACE, ...args], {
    encoding: 'utf8',
  })
}

interface HelmRev {
  revision: number
  status: string
  description: string
}
function history(release: string): HelmRev[] {
  return JSON.parse(helm(['history', release, '-o', 'json'])) as HelmRev[]
}
function kubectlGet(args: readonly string[]): string {
  return execFileSync(
    'kubectl',
    ['--context', liveContext(), '-n', LIVE_NAMESPACE, 'get', ...args],
    {
      encoding: 'utf8',
    },
  )
}
function deploymentVersion(name: string): string {
  return kubectlGet(['deployment', name, '-o', 'jsonpath={.metadata.resourceVersion}'])
}

function podTemplate(name: string): string {
  return kubectlGet(['deployment', name, '-o', 'jsonpath={.spec.template.spec.containers[0].env}'])
}

async function openRelease(page: Page, name: string): Promise<void> {
  await navigateTo(page, 'Helm Releases', 'Overview')
  await row(page, `${LIVE_NAMESPACE}/${name}`, HELM).click()
  await expect(inspector(page)).toContainText(name)
}

const BARE_CHART = 'oci://ghcr.io/stefanprodan/charts/podinfo'
const BARE_VERSION = '6.15.0'

/** The two revisions `scripts/sandbox.sh` installs: rev 1 plain, rev 2 with a message. */
function installBare(): void {
  const base = ['--version', BARE_VERSION, '--set', 'replicaCount=1']
  helm(['upgrade', '--install', BARE, BARE_CHART, ...base, '--wait'])
  helm(['upgrade', BARE, BARE_CHART, ...base, '--set', 'ui.message=qs006-before', '--wait'])
}

/** Whether `quickstart-bare` is at the fixture's baseline, rather than mid-scenario or gone. */
function bareIsBaseline(): boolean {
  try {
    const values = JSON.parse(helm(['get', 'values', BARE, '-o', 'json'])) as {
      replicaCount?: number
      ui?: { message?: string }
    }
    return (
      values.replicaCount === 1 &&
      values.ui?.message === 'qs006-before' &&
      history(BARE).length >= 2
    )
  } catch {
    return false
  }
}

test.describe('live 006 quickstart', () => {
  // S3 uninstalls the bare release and S1/S2 move it off its baseline, so the second project —
  // and every re-run — used to find no release at all and fail on `release: not found`. Put the
  // fixture back before each project's scenarios instead of asking a human to remember.
  test.beforeAll(() => {
    if (bareIsBaseline()) return
    try {
      helm(['uninstall', BARE, '--wait'])
    } catch {
      // Not installed; nothing to remove.
    }
    installBare()
  })

  test.beforeEach(async ({ app }) => {
    await app.open()
    await app.selectContext()
    await app.selectNamespace(LIVE_NAMESPACE)
  })

  test('S1 rollback: preview mutates nothing, cancel adds no revision, confirm adds one', async ({
    page,
  }) => {
    const before = history(BARE)
    expect(before.length).toBeGreaterThanOrEqual(2)
    const rv = deploymentVersion(`${BARE}-podinfo`)
    const manifestBefore = helm(['get', 'manifest', BARE])
    const tplBefore = podTemplate(`${BARE}-podinfo`)
    await openRelease(page, BARE)
    const insp = inspector(page)

    // Preview, then cancel at the preview step.
    await insp.getByTestId('helm-rollback-1').click()
    await expect(insp.getByTestId('yaml-diff')).toBeVisible()
    await expect(insp.getByTestId('helm-rollback-gitops-warning')).toHaveCount(0)
    expect(deploymentVersion(`${BARE}-podinfo`)).toBe(rv)
    await insp.getByRole('button', { name: 'Cancel', exact: true }).click()
    expect(history(BARE).length).toBe(before.length)

    // Preview, open the confirm dialog, cancel there.
    await insp.getByTestId('helm-rollback-1').click()
    await insp.getByTestId('helm-rollback-confirm-open').click()
    await expect(confirmDialog(page)).toBeVisible()
    await expect(confirmDialog(page)).not.toContainText('Managed by')
    await confirmDialog(page).getByTestId('confirm-cancel').click()
    expect(history(BARE).length).toBe(before.length)
    expect(deploymentVersion(`${BARE}-podinfo`)).toBe(rv)

    // Confirm for real.
    await insp.getByTestId('helm-rollback-confirm-open').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect(insp).toContainText('Rolled back.')
    const after = history(BARE)
    expect(after.length).toBe(before.length + 1)
    expect(after.at(-1)?.description).toMatch(/Rollback to 1/)
    const rev1 = helm(['get', 'manifest', BARE, '--revision', '1'])
    const now = helm(['get', 'manifest', BARE])
    expect(now).toBe(rev1)
    // The rollback reached the cluster, not only Helm's history: exactly one revision is
    // `deployed`, and the live Deployment's pod template is rev 1's again.
    expect(history(BARE).filter((r) => r.status === 'deployed')).toHaveLength(1)
    if (manifestBefore !== rev1) {
      await expect
        .poll(() => podTemplate(`${BARE}-podinfo`), { timeout: 60_000 })
        .not.toBe(tplBefore)
    }
  })

  test('S1.6 rollback of the Flux-managed release warns and links the HelmRelease', async ({
    page,
  }) => {
    const before = history(FLUX)
    expect(before.length).toBeGreaterThanOrEqual(2)
    await openRelease(page, FLUX)
    const insp = inspector(page)
    await insp.getByTestId('helm-rollback-1').click()
    const warning = insp.getByTestId('helm-rollback-gitops-warning')
    await expect(warning).toBeVisible()
    await expect(warning).toContainText('Flux')
    await expect(warning).toContainText(FLUX)
    await insp.getByTestId('helm-rollback-confirm-open').click()
    await expect(confirmDialog(page)).toContainText('Managed by Flux')
    await confirmDialog(page).getByTestId('confirm-cancel').click()
    await insp.getByRole('button', { name: 'Cancel', exact: true }).click()
    expect(history(FLUX).length).toBe(before.length)
  })

  test('S2 upgrade: effective values preload, invalid YAML errors, preview then apply', async ({
    page,
  }) => {
    const before = history(BARE)
    await openRelease(page, BARE)
    const insp = inspector(page)
    await insp.getByTestId('helm-upgrade-tab').click()
    const values = insp.getByTestId('helm-upgrade-values')
    await expect(values).toHaveValue(/replicaCount: 1/)
    const original = await values.inputValue()

    // Invalid YAML: a clear error at preview, nothing to apply, no revision.
    await values.fill('not: [valid')
    await insp.getByTestId('helm-upgrade-preview').click()
    await expect(insp.getByTestId('helm-upgrade-error')).toBeVisible()
    await expect(insp.getByTestId('helm-upgrade-apply')).toHaveCount(0)
    expect(history(BARE).length).toBe(before.length)

    // Valid edit: preview is a dry run (no revision), then apply.
    const rv = deploymentVersion(`${BARE}-podinfo`)
    await values.fill(original.replace(/replicaCount: 1\b/, 'replicaCount: 2'))
    await insp.getByTestId('helm-upgrade-preview').click()
    const diff = insp.getByTestId('yaml-diff')
    await expect(diff).toBeVisible({ timeout: 60_000 })
    await expect(diff.getByTestId('diff-added').filter({ hasText: 'replicas: 2' })).toHaveCount(1)
    await expect(insp.getByTestId('helm-upgrade-gitops-warning')).toHaveCount(0)
    expect(history(BARE).length).toBe(before.length)
    expect(deploymentVersion(`${BARE}-podinfo`)).toBe(rv)

    await insp.getByTestId('helm-upgrade-apply').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect(insp.getByTestId('helm-upgrade-applied')).toBeVisible({ timeout: 90_000 })
    expect(history(BARE).length).toBe(before.length + 1)
    expect(kubectlGet(['deployment', `${BARE}-podinfo`, '-o', 'jsonpath={.spec.replicas}'])).toBe(
      '2',
    )
  })

  test('S2.5 upgrade of the Flux-managed release warns before the diff', async ({ page }) => {
    const before = history(FLUX)
    await openRelease(page, FLUX)
    const insp = inspector(page)
    await insp.getByTestId('helm-upgrade-tab').click()
    const values = insp.getByTestId('helm-upgrade-values')
    await expect(values).toHaveValue(/replicaCount/)
    await insp.getByTestId('helm-upgrade-preview').click()
    await expect(insp.getByTestId('helm-upgrade-gitops-warning')).toBeVisible({ timeout: 60_000 })
    await expect(insp.getByTestId('helm-upgrade-gitops-warning')).toContainText('Flux')
    // Never apply against the Flux-owned release.
    expect(history(FLUX).length).toBe(before.length)
  })

  test('S3 uninstall: Flux release warns (cancelled); bare release cancel then confirm', async ({
    page,
  }) => {
    await openRelease(page, FLUX)
    await inspector(page).getByTestId('helm-actions-tab').click()
    await inspector(page).getByTestId('helm-release-uninstall').click()
    await expect(confirmDialog(page).getByTestId('helm-uninstall-gitops-warning')).toBeVisible()
    await confirmDialog(page).getByTestId('confirm-cancel').click()
    expect(history(FLUX).length).toBeGreaterThanOrEqual(1)

    await openRelease(page, BARE)
    const insp = inspector(page)
    await insp.getByTestId('helm-actions-tab').click()
    await insp.getByTestId('helm-release-uninstall').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toContainText(`${LIVE_NAMESPACE}/${BARE}`)
    await expect(dialog.getByTestId('helm-uninstall-gitops-warning')).toHaveCount(0)
    await expect(dialog.getByRole('textbox')).toHaveCount(0)
    await dialog.getByTestId('confirm-cancel').click()
    expect(helm(['status', BARE, '-o', 'json'])).toContain('"deployed"')

    await insp.getByTestId('helm-release-uninstall').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect(row(page, `${LIVE_NAMESPACE}/${BARE}`, HELM)).toHaveCount(0, { timeout: 60_000 })
    expect(helm(['list', '-q'])).not.toContain(BARE)
    await expect
      .poll(
        () => kubectlGet(['all', '-l', `app.kubernetes.io/instance=${BARE}`, '-o', 'name']).trim(),
        { timeout: 60_000 },
      )
      .toBe('')
  })
})
