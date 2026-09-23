/**
 * US5 (011-rbac-oidc-administration, T048-T049): the standalone OIDC issuer config generator
 * panel (`ui/src/resources/oidcConfig.tsx`) against the fake backend. This is a config
 * GENERATOR, never a mutation: the only command it ever sends is `RefreshKubeadmDetection`
 * (best-effort informational probe), and it must never render anything resembling the
 * diff-preview/apply flow (`YamlDiffView`'s `yaml-diff` test id).
 *
 * Navigation: `access/oidc-config` is registered in `ui/src/resources/nav.ts` under the existing
 * "Access Control" group (label assumed "OIDC Config") and the `PaneHost` rendering branch wired
 * by a separate integration pass (tasks.md T041/T042-equivalent for this section) that lands
 * after this spec is written — mirrored from `us4-rbac-namespace-access.spec.ts`'s identical
 * note for `access/namespace-access`. `navigateTo` is the same helper every other nav-driven spec
 * in this suite uses, so this should work once that wiring lands; if the integration pass gives
 * the nav label something other than "OIDC Config", only the `navigateTo` call here needs
 * updating.
 */
import { expect, type Page, test } from '@playwright/test'

import type { KubeadmDetectionResult } from '../../src/protocol/types'
import { batch, globalRoute, read } from '../fixtures/builders'
import { emit, navigateTo, open, sent } from './helpers'

const SECTION = 'access/oidc-config'

async function openGenerator(page: Page) {
  await open(page)
  await navigateTo(page, 'OIDC Config', 'Access Control')
  const view = page.getByTestId('rbac-oidc-generator-panel')
  await expect(view).toBeVisible()
  return view
}

async function fillForm(page: Page, view: ReturnType<typeof page.getByTestId>) {
  await view.getByTestId('rbac-oidc-issuer-url').getByRole('textbox').fill('https://accounts.example.com')
  await view.getByTestId('rbac-oidc-client-id').getByRole('textbox').fill('fathom-cluster')
  await view.getByTestId('rbac-oidc-audience').getByRole('textbox').fill('https://kubernetes.example.com')
}

test.describe('US5 OIDC config generator', () => {
  test('generating renders a distinct output panel, never the diff view', async ({ page }) => {
    const view = await openGenerator(page)
    await fillForm(page, view)

    await view.getByTestId('rbac-oidc-generate').click()

    const output = view.getByTestId('rbac-oidc-generator-output')
    await expect(output).toBeVisible()

    // Structurally distinct from the mutation-preview flow: never the diff view's element, and
    // no Apply/Confirm affordance anywhere on the page.
    await expect(page.getByTestId('yaml-diff')).toHaveCount(0)
    await expect(page.getByTestId('diff-added')).toHaveCount(0)
    await expect(page.getByTestId('diff-removed')).toHaveCount(0)
    await expect(page.getByRole('button', { name: /^apply$/i })).toHaveCount(0)
    await expect(page.getByTestId('confirm-submit')).toHaveCount(0)

    // The disclaimer is visible once generated, not buried.
    await expect(output).toContainText(
      'Fathom does not modify the API server',
    )
    await expect(page.getByTestId('rbac-oidc-generator-disclaimer')).toBeVisible()

    // Flags rendered as a code block, and provider-labeled instructions for EKS/GKE/AKS.
    const flags = output.getByTestId('rbac-oidc-generator-flags')
    await expect(flags).toContainText('--oidc-issuer-url=https://accounts.example.com')
    await expect(flags).toContainText('--oidc-client-id=fathom-cluster')
    await expect(flags).toContainText('--oidc-required-claim=aud=https://kubernetes.example.com')
    await expect(output.getByTestId('rbac-oidc-provider-eks')).toBeVisible()
    await expect(output.getByTestId('rbac-oidc-provider-gke')).toBeVisible()
    await expect(output.getByTestId('rbac-oidc-provider-aks')).toBeVisible()
  })

  test('copy button copies the flags to the clipboard', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])
    const view = await openGenerator(page)
    await fillForm(page, view)
    await view.getByTestId('rbac-oidc-generate').click()

    const output = view.getByTestId('rbac-oidc-generator-output')
    const copyButton = output.getByTestId('rbac-oidc-generator-copy')
    await expect(copyButton).toBeEnabled()
    await copyButton.click()

    const clipboardText = await page.evaluate(() => navigator.clipboard.readText())
    expect(clipboardText).toContain('--oidc-issuer-url=https://accounts.example.com')
  })

  test('export button downloads a file with the flags and instructions', async ({ page }) => {
    const view = await openGenerator(page)
    await fillForm(page, view)
    await view.getByTestId('rbac-oidc-generate').click()

    const output = view.getByTestId('rbac-oidc-generator-output')
    const exportButton = output.getByTestId('rbac-oidc-generator-export')
    await expect(exportButton).toBeEnabled()

    const [download] = await Promise.all([page.waitForEvent('download'), exportButton.click()])
    expect(download.suggestedFilename()).toBe('fathom-oidc-config.txt')
    const stream = await download.createReadStream()
    const chunks: Buffer[] = []
    for await (const chunk of stream) chunks.push(chunk as Buffer)
    const text = Buffer.concat(chunks).toString('utf-8')
    expect(text).toContain('--oidc-issuer-url=https://accounts.example.com')
    expect(text).toContain('EKS')
    expect(text).toContain('GKE')
    expect(text).toContain('AKS')
  })

  test('generation works before and regardless of the kubeadm-detection reply timing', async ({
    page,
  }) => {
    const view = await openGenerator(page)
    // Generate immediately, without ever answering RefreshKubeadmDetection — must not block.
    await fillForm(page, view)
    await view.getByTestId('rbac-oidc-generate').click()
    await expect(view.getByTestId('rbac-oidc-generator-output')).toBeVisible()

    // Now answer the detection probe late; the badge updates but nothing about generation
    // depends on it having replied.
    const result: KubeadmDetectionResult = { detected: true, checked: true }
    await emit(
      page,
      batch([read(900, globalRoute(SECTION), { type: 'KubeadmDetectionUpdated', data: result })]),
    )
    await expect(view.getByTestId('rbac-oidc-generator-kubeadm-badge')).toContainText(
      'Kubeadm cluster detected',
    )
  })

  test('only RefreshKubeadmDetection (plus read-only bootstrap) is ever sent — no cluster writes', async ({
    page,
  }) => {
    const view = await openGenerator(page)
    await fillForm(page, view)
    await view.getByTestId('rbac-oidc-generate').click()
    await expect(view.getByTestId('rbac-oidc-generator-output')).toBeVisible()

    const exportButton = view.getByTestId('rbac-oidc-generator-export')
    await Promise.all([page.waitForEvent('download'), exportButton.click()])

    const types = (await sent(page)).map((c) => c.type)
    expect(types).toContain('RefreshKubeadmDetection')
    for (const writeType of [
      'ApplyResourceYaml',
      'CreateResourceYaml',
      'DeleteResource',
      'DeleteResources',
      'StartPortForward',
      'HelmUpgrade',
      'HelmRollback',
      'HelmUninstall',
    ]) {
      expect(types).not.toContain(writeType)
    }
  })
})
