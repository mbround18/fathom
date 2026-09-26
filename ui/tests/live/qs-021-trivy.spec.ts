/**
 * LIVE quickstart validation for 021-trivy-vulnerability-pane (T045), against the real Trivy
 * Operator in the live cluster (k8s/addons/trivy). Read-only apart from the opt-in US3 test.
 *
 * - US1: a `web` Pod's Vulnerabilities tab matches the counts in its owner's VulnerabilityReports;
 *   the never-pulled `bad-image` Pod shows the honest "no scan results yet" state.
 * - US2: the cluster pane is ranked worst-first and its `web` row agrees with US1; a row opens
 *   that workload's Inspector.
 * - US3 (only with FATHOM_QS021_UNINSTALL=1 on a `kind-*` context): deletes the Trivy CRDs, checks
 *   the "not installed" state on both surfaces, re-creates them, and checks the pane goes live
 *   again without a restart. Reports are re-created by the operator over the next minutes.
 */
import { execFileSync } from 'node:child_process'
import { readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import type { Page } from '@playwright/test'

import { liveContext } from './env'
import {
  expect,
  inspector,
  kubectlJson,
  LIVE_NAMESPACE,
  navigateTo,
  PODS,
  row,
  search,
  test,
} from './helpers'

const SEVERITIES = ['CRITICAL', 'HIGH', 'MEDIUM', 'LOW', 'UNKNOWN'] as const
type Counts = Record<(typeof SEVERITIES)[number], number>

interface Report {
  metadata: { labels?: Record<string, string> }
  report: {
    summary: {
      criticalCount: number
      highCount: number
      mediumCount: number
      lowCount: number
      unknownCount: number
    }
  }
}

function podOf(app: string): { name: string; rs: string } {
  const pod = kubectlJson<{
    items: { metadata: { name: string; ownerReferences?: { kind: string; name: string }[] } }[]
  }>(['get', 'pods', '-l', `app=${app}`, '-o', 'json']).items[0]
  if (!pod) throw new Error(`no pod for app=${app}`)
  const rs = pod.metadata.ownerReferences?.find((o) => o.kind === 'ReplicaSet')?.name ?? ''
  return { name: pod.metadata.name, rs }
}

/** Sum of every report for `ReplicaSet/<rs>` (one per container image). */
function kubectlCounts(rs: string): Counts {
  // Not via kubectl(): its allow-list has only core namespaced kinds. Read-only, fathom-dev.
  const out = execFileSync(
    'kubectl',
    [
      '--context',
      liveContext(),
      '-n',
      LIVE_NAMESPACE,
      'get',
      'vulnerabilityreports.aquasecurity.github.io',
      '-l',
      `trivy-operator.resource.kind=ReplicaSet,trivy-operator.resource.name=${rs}`,
      '-o',
      'json',
    ],
    { encoding: 'utf8' },
  )
  const reports = (JSON.parse(out) as { items: Report[] }).items
  const c: Counts = { CRITICAL: 0, HIGH: 0, MEDIUM: 0, LOW: 0, UNKNOWN: 0 }
  for (const r of reports) {
    const s = r.report.summary
    c.CRITICAL += s.criticalCount
    c.HIGH += s.highCount
    c.MEDIUM += s.mediumCount
    c.LOW += s.lowCount
    c.UNKNOWN += s.unknownCount
  }
  return c
}

async function openPodVulns(page: Page, name: string) {
  await navigateTo(page, 'Pods')
  await search(page, name, PODS)
  await row(page, `${LIVE_NAMESPACE}/${name}`, PODS).click()
  const insp = inspector(page)
  await expect(insp.getByRole('heading', { name })).toBeVisible()
  await insp.getByTestId('vulnerabilities-tab').click()
  return insp
}

async function openPane(page: Page) {
  await navigateTo(page, 'Vulnerabilities', 'Integrations')
}

function parseRow(text: string) {
  const m = /C (\d+)\s*H (\d+)\s*M (\d+)\s*L (\d+)/.exec(text)
  if (!m) throw new Error(`unparsable trivy row: ${text}`)
  return [m[1], m[2], m[3], m[4]].map(Number) as [number, number, number, number]
}

test.describe('live 021 quickstart', () => {
  test.beforeEach(async ({ app }) => {
    await app.open()
    await app.selectContext()
    await app.selectNamespace(LIVE_NAMESPACE)
  })

  test('US1 Inspector counts match the VulnerabilityReports; unscanned Pod is honest', async ({
    page,
  }) => {
    const web = podOf('web')
    const want = kubectlCounts(web.rs)
    expect(Object.values(want).reduce((a, b) => a + b, 0)).toBeGreaterThan(0)
    const insp = await openPodVulns(page, web.name)
    for (const sev of SEVERITIES) {
      await expect(insp.getByTestId(`vuln-count-${sev}`)).toHaveText(String(want[sev]), {
        timeout: 30_000,
      })
    }
    await expect(insp.getByTestId('vuln-finding-row').first()).toBeVisible()
    const first = (await insp.getByTestId('vuln-finding-row').first().textContent()) ?? ''
    test.info().annotations.push({ type: 'web counts', description: JSON.stringify(want) })
    test.info().annotations.push({ type: 'top finding', description: first })
    expect(first).toMatch(/CVE-\d{4}-\d+/)

    const bad = podOf('bad-image')
    const insp2 = await openPodVulns(page, bad.name)
    await expect(insp2.getByTestId('vuln-error-NoReportYet')).toBeVisible({ timeout: 30_000 })
    await expect(insp2.getByTestId('vuln-count-CRITICAL')).toHaveCount(0)
  })

  test('US2 cluster pane is ranked worst-first, agrees with US1, rows open the Inspector', async ({
    page,
  }) => {
    const web = podOf('web')
    const want = kubectlCounts(web.rs)
    await openPane(page)
    const rows = page.getByTestId('trivy-workload-row')
    await expect(rows.first()).toBeVisible({ timeout: 60_000 })
    const texts = await rows.allTextContents()
    test.info().annotations.push({ type: 'rows', description: String(texts.length) })
    test.info().annotations.push({ type: 'top 3', description: texts.slice(0, 3).join(' | ') })
    const parsed = texts.map(parseRow)
    for (let i = 1; i < parsed.length; i++) {
      const a = parsed[i - 1] ?? [0, 0, 0, 0]
      const b = parsed[i] ?? [0, 0, 0, 0]
      // Lexicographic (C, H, M, L) non-increasing.
      const cmp = a.findIndex((v, j) => v !== b[j])
      if (cmp >= 0) expect(a[cmp]).toBeGreaterThan(b[cmp] ?? 0)
    }
    const webRow = texts.find(
      (t) => t.includes(`${LIVE_NAMESPACE}/web `) || / fathom-dev\/web\b/.test(t),
    )
    expect(webRow, 'fathom-dev/web row').toBeDefined()
    expect(parseRow(webRow ?? '')).toEqual([want.CRITICAL, want.HIGH, want.MEDIUM, want.LOW])

    const top = rows.first()
    const topText = (await top.textContent()) ?? ''
    // The first span is "<Kind> <ns>/<name>"; the counts follow in their own spans.
    const topLabel = (await top.locator('span').first().textContent()) ?? ''
    const topName = /\/(\S+)$/.exec(topLabel.trim())?.[1] ?? ''
    expect(topName).not.toBe('')
    await top.click()
    await expect(inspector(page)).toBeVisible()
    await expect(inspector(page).getByRole('heading', { name: topName })).toBeVisible()
    const tabs = await inspector(page).getByRole('tab').allTextContents()
    test.info().annotations.push({
      type: 'top row opens',
      description: `${topText} -> tabs ${tabs.join(',')}`,
    })
  })

  test('US3 not installed → honest state on both surfaces; CRDs back → live again', async ({
    page,
  }) => {
    test.skip(
      process.env.FATHOM_QS021_UNINSTALL !== '1' || !liveContext().startsWith('kind-'),
      'opt-in (FATHOM_QS021_UNINSTALL=1) and kind-only: deletes the Trivy CRDs',
    )
    const crdDir = fileURLToPath(
      new URL(
        '../../../k8s/addons/trivy/charts/trivy-operator-0.36.0/trivy-operator/crds/',
        import.meta.url,
      ),
    )
    const crds = readdirSync(crdDir).filter((f) => f.endsWith('.yaml'))
    const names = crds.map(
      (f) =>
        f.replace(/^aquasecurity\.github\.io_/, '').replace(/\.yaml$/, '') +
        '.aquasecurity.github.io',
    )
    const k = (args: string[]) =>
      execFileSync('kubectl', ['--context', liveContext(), ...args], { encoding: 'utf8' })

    const web = podOf('web')
    k(['delete', 'crd', ...names, '--wait=true'])
    try {
      const insp = await openPodVulns(page, web.name)
      await expect(insp.getByTestId('vuln-error-NotInstalled')).toBeVisible({ timeout: 30_000 })
      await openPane(page)
      await expect(page.getByText(/CRDs were not found/)).toBeVisible({ timeout: 30_000 })
    } finally {
      k(['apply', '--server-side', '--force-conflicts', '-f', crdDir])
      k(['-n', 'trivy-system', 'rollout', 'restart', 'deployment/trivy-operator'])
    }
    // Live detection: the open pane leaves "not installed" within one poll (20s) + slack. Scoped
    // to the pane: the Pod Inspector's one-shot tab still shows its earlier NotInstalled result.
    const pane = page.getByRole('region', { name: 'Vulnerabilities' })
    await expect(pane.getByText(/CRDs were not found/)).toHaveCount(0, { timeout: 30_000 })
    await expect(
      pane.getByTestId('trivy-empty').or(pane.getByTestId('trivy-workload-row').first()),
    ).toBeVisible()
  })
})
