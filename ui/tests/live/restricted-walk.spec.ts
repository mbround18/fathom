/**
 * LIVE, read-only, as a NAMESPACE-SCOPED user (k8s/samples/restricted-user.yaml: edit in
 * fathom-dev and demo, view in default, nothing cluster-scoped). Run through
 * playwright.restricted.config.ts; skipped unless FATHOM_RESTRICTED_KUBECONFIG is set.
 *
 * Every Sections-nav entry and nav tool (not Fleet: it polls every context) must either work or
 * show a clean "Not permitted" state. Broken is: a pane still loading, an error pane or inline
 * error, a blank pane, placeholder text, a page error, or ANY error notification from the
 * backend while the walk runs (a 403 must never surface as an error toast or repeat as noise).
 * The admin twin is tests/live/nav-walk.spec.ts.
 */
import type { Locator, Page, WebSocketRoute } from '@playwright/test'

import type { DevBridgeServerMsg } from '../../src/protocol/types'
import { NAV_GROUPS, NAV_TOOLS } from '../../src/resources/nav'

import { restrictedKubeconfig } from './env'
import { LIVE_NAMESPACE, expect, test } from './helpers'

const STUB =
  /section unavailable|coming soon|not implemented|under construction|\bTODO\b|lorem ipsum/i
const SETTLE_MS = 20_000
/** Late errors (a background poller's first tick) get this long to show up per item. */
const GRACE_MS = 1_500

type Outcome = 'works' | 'not permitted' | 'broken'
interface Result {
  item: string
  outcome: Outcome
  detail: string
}

/** Error notifications the backend sent (bridge frames), recorded before the UI sees them. */
async function tapErrors(page: Page): Promise<string[]> {
  const errors: string[] = []
  await page.routeWebSocket(/^ws:\/\/127\.0\.0\.1:\d+\//, (ws: WebSocketRoute) => {
    const server = ws.connectToServer()
    ws.onMessage((raw) => server.send(raw))
    server.onMessage((raw) => {
      const msg = JSON.parse(String(raw)) as DevBridgeServerMsg
      if (msg.op === 'batch') {
        for (const env of msg.batch.envelopes) {
          const ev = env.event
          if (
            ev.type === 'System' &&
            ev.data.type === 'Notification' &&
            ev.data.data.level === 'Error'
          ) {
            errors.push(ev.data.data.message)
          }
        }
      }
      ws.send(raw)
    })
  })
  return errors
}

async function pick(page: Page, selector: string) {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  await expect(nav).toBeVisible()
  const collapsed = nav.locator('button[aria-expanded="false"]')
  while ((await collapsed.count()) > 0) await collapsed.first().click()
  await nav.locator(selector).click()
}

async function classify(
  page: Page,
  region: Locator,
  item: string,
  errors: string[],
  pageErrors: string[],
  errorsBefore: number,
): Promise<Result> {
  const broken = (detail: string): Result => ({ item, outcome: 'broken', detail })
  if (!(await region.isVisible())) return broken('pane not visible')
  try {
    await expect(region.locator('[data-pane-state="loading"]')).toHaveCount(0, {
      timeout: SETTLE_MS,
    })
  } catch {
    return broken('stuck loading')
  }
  await page.waitForTimeout(GRACE_MS)
  const newErrors = errors.slice(errorsBefore)
  if (newErrors.length > 0) {
    return broken(`${String(newErrors.length)} error notice(s): ${newErrors[0] ?? ''}`)
  }
  if (pageErrors.length > 0) return broken(`page error: ${pageErrors.join('; ')}`)
  const errorPane = region.locator('[data-pane-state="error"]')
  if ((await errorPane.count()) > 0) {
    return broken(`error pane: ${(await errorPane.first().innerText()).replace(/\s+/g, ' ')}`)
  }
  const alert = region.locator('[role="alert"]')
  if ((await alert.count()) > 0 && (await alert.first().isVisible())) {
    return broken(`inline error: ${(await alert.first().innerText()).replace(/\s+/g, ' ')}`)
  }
  const text = (await region.innerText()).trim()
  if (text === '') return broken('blank pane')
  if (STUB.test(text)) return broken('placeholder text')
  if ((await region.locator('[data-pane-state="not-permitted"]').count()) > 0) {
    return { item, outcome: 'not permitted', detail: '' }
  }
  return { item, outcome: 'works', detail: '' }
}

function report(results: readonly Result[]): string {
  return results
    .map((r) => `${r.outcome.padEnd(13)} ${r.item}${r.detail ? `  (${r.detail})` : ''}`)
    .join('\n')
}

test.describe('restricted-user walk', () => {
  test.skip(!restrictedKubeconfig(), 'FATHOM_RESTRICTED_KUBECONFIG is not set')

  for (const group of NAV_GROUPS) {
    test(`${group.label}: works or cleanly not permitted`, async ({ app, page }, info) => {
      test.setTimeout(group.sections.length * (SETTLE_MS + GRACE_MS + 5_000) + 60_000)
      const errors = await tapErrors(page)
      const pageErrors: string[] = []
      page.on('pageerror', (e) => pageErrors.push(e.message))
      await app.open()
      const results: Result[] = []
      for (const s of group.sections) {
        const before = errors.length
        pageErrors.length = 0
        await pick(page, `[data-section="${s.key}"]`)
        const pane = page.locator(`section[data-slot="pane"][data-section="${s.key}"]`).first()
        results.push(await classify(page, pane, s.key, errors, pageErrors, before))
      }
      await info.attach('results', { body: report(results), contentType: 'text/plain' })
      console.log(`\n[${group.label}]\n${report(results)}`)
      expect(results.filter((r) => r.outcome === 'broken')).toEqual([])
    })
  }

  test('nav tools: works or cleanly not permitted', async ({ app, page }, info) => {
    test.setTimeout(NAV_TOOLS.length * (SETTLE_MS + GRACE_MS + 5_000) + 60_000)
    const errors = await tapErrors(page)
    const pageErrors: string[] = []
    page.on('pageerror', (e) => pageErrors.push(e.message))
    await app.open()
    const results: Result[] = []
    for (const tool of NAV_TOOLS) {
      const before = errors.length
      pageErrors.length = 0
      await pick(page, `[data-tool="${tool}"]`)
      const region = page.getByTestId('tools-sheet')
      results.push(await classify(page, region, `tool:${tool}`, errors, pageErrors, before))
      await page.keyboard.press('Escape')
      await expect(region).toBeHidden()
    }
    await info.attach('results', { body: report(results), contentType: 'text/plain' })
    console.log(`\n[tools]\n${report(results)}`)
    expect(results.filter((r) => r.outcome === 'broken')).toEqual([])
  })

  test('namespace picker falls back to the context namespace and accepts a typed one', async ({
    app,
    page,
  }) => {
    const errors = await tapErrors(page)
    await app.open()
    // The context's own namespace is offered although namespaces cannot be listed.
    await app.selectNamespace(LIVE_NAMESPACE)
    await expect(page.getByRole('combobox', { name: 'Namespace' })).toContainText(LIVE_NAMESPACE)
    // A namespace the user can reach but the picker cannot know about can be typed in.
    await page.getByRole('combobox', { name: 'Namespace' }).click()
    await page.getByPlaceholder('Find namespace…').fill('demo')
    await page.getByRole('option', { name: /use namespace.*demo/i }).click()
    await expect(page.getByRole('combobox', { name: 'Namespace' })).toContainText('demo')
    await pick(page, '[data-section="workloads/deployments"]')
    const pane = page
      .locator('section[data-slot="pane"][data-section="workloads/deployments"]')
      .first()
    await expect(pane.locator('[data-pane-state="loading"]')).toHaveCount(0, {
      timeout: SETTLE_MS,
    })
    await expect(pane.locator('[data-pane-state="error"]')).toHaveCount(0)
    expect(errors).toEqual([])
  })
})
