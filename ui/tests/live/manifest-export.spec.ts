/**
 * LIVE (040-manifest-export, T038): what Fathom exports can be re-applied. Downloads a clean
 * Deployment from the fathom-dev sandbox and a clean two-object bundle, then has the API server
 * validate each file with `kubectl apply --dry-run=server` (nothing is persisted). Also exports
 * the bridge's own kubeconfig context to a temporary file and checks it is owner-only (0600)
 * and usable. Only the pinned live context is ever touched.
 */
import { execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import type { Page } from '@playwright/test'

import {
  checkRows,
  DEPLOYMENTS,
  expect,
  findRow,
  inspector,
  isPhone,
  LIVE_NAMESPACE,
  showList,
  test,
} from './helpers'

/** `kubectl apply --dry-run=server -f <file>` against the live context only. */
function dryRunApply(context: string, file: string): string {
  if (context === 'default') throw new Error('refusing the default (production) context')
  return execFileSync('kubectl', ['--context', context, 'apply', '--dry-run=server', '-f', file], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    timeout: 60_000,
  })
}

async function download(page: Page, click: () => Promise<void>, dir: string): Promise<string> {
  const [d] = await Promise.all([page.waitForEvent('download'), click()])
  const path = join(dir, d.suggestedFilename())
  await d.saveAs(path)
  return path
}

test.describe('040 manifest export (live)', () => {
  let dir = ''
  test.beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), 'fathom-040-'))
  })
  test.afterEach(() => {
    rmSync(dir, { recursive: true, force: true })
  })

  test('a clean Deployment export re-applies (server dry run)', async ({ app, page }) => {
    await app.open()
    await app.selectNamespace(LIVE_NAMESPACE)
    await showList(page, DEPLOYMENTS, 'Deployments')
    const r = await findRow(page, 'web', DEPLOYMENTS)
    await r.click()
    const insp = inspector(page)
    await insp.getByRole('tab', { name: 'YAML' }).click()
    const bar = insp.getByTestId('manifest-actions')
    await expect(bar.getByTestId('manifest-download')).toBeEnabled()
    await expect(bar.getByTestId('manifest-clean')).toHaveAttribute('aria-checked', 'true')
    const file = await download(page, () => bar.getByTestId('manifest-download').click(), dir)
    expect(file).toMatch(/deployment-fathom-dev-web\.yaml$/)

    const yaml = readFileSync(file, 'utf8')
    expect(yaml).toContain('kind: Deployment')
    expect(yaml).not.toMatch(/resourceVersion|uid:|managedFields|creationTimestamp|^status:/m)
    const out = dryRunApply(app.context, file)
    expect(out).toMatch(/deployment\.apps\/web (configured|unchanged) \(server dry run\)/)
  })

  test('a clean two-Deployment bundle re-applies (server dry run)', async ({ app, page }) => {
    await app.open()
    await app.selectNamespace(LIVE_NAMESPACE)
    await showList(page, DEPLOYMENTS, 'Deployments')
    // Selection survives narrowing the list, so tick each through its own search.
    await checkRows(page, ['web'], { section: DEPLOYMENTS })
    await checkRows(page, ['scale-me'], { section: DEPLOYMENTS })
    await page.getByTestId('bulk-export').click()
    const dialog = page.getByTestId('manifest-export-dialog')
    await expect(dialog.getByTestId('manifest-export-count')).toHaveText('2 of 2 objects')
    const file = await download(page, () => dialog.getByTestId('manifest-download').click(), dir)
    expect(readFileSync(file, 'utf8').split('\n---\n')).toHaveLength(2)
    const out = dryRunApply(app.context, file)
    expect(out).toMatch(/deployment\.apps\/scale-me (configured|unchanged) \(server dry run\)/)
    expect(out).toMatch(/deployment\.apps\/web (configured|unchanged) \(server dry run\)/)
  })

  test('kubeconfig export writes an owner-only file that works', async ({ app, page }, info) => {
    await app.open()
    if (isPhone(info)) await page.getByTestId('scope-chip').click()
    await page.getByTestId('kubeconfig-export-open').click()
    const dialog = page.getByTestId('kubeconfig-export-dialog')
    await expect(dialog.getByTestId('kubeconfig-context')).toHaveValue(app.context)
    await expect(dialog.getByTestId('kubeconfig-preview')).toContainText(`name: ${app.context}`)
    // Nothing secret reaches the page: kind users embed a client key, shown redacted.
    await expect(dialog.getByTestId('kubeconfig-preview')).not.toContainText('BEGIN')
    await expect(dialog.getByTestId('kubeconfig-embedded')).toBeVisible()

    const path = join(dir, 'exported.kubeconfig')
    await dialog.getByTestId('kubeconfig-path').fill(path)
    await dialog.getByTestId('kubeconfig-include-credentials').click()
    await dialog.getByTestId('kubeconfig-export').click()
    await page.getByTestId('confirm-dialog').getByTestId('confirm-submit').click()
    await expect(dialog.getByTestId('kubeconfig-result')).toHaveAttribute('data-ok', 'true')

    expect(statSync(path).mode & 0o777).toBe(0o600)
    const ns = execFileSync(
      'kubectl',
      ['--kubeconfig', path, 'get', 'namespace', LIVE_NAMESPACE, '-o', 'name'],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 60_000 },
    )
    expect(ns.trim()).toBe(`namespace/${LIVE_NAMESPACE}`)
  })
})
