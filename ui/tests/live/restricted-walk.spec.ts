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

import type {
  ApiServerAuthComparison,
  BackendCmd,
  DevBridgeServerMsg,
} from '../../src/protocol/types'
import { NAV_GROUPS, NAV_TOOLS } from '../../src/resources/nav'

import { liveContext, restrictedKubeconfig } from './env'
import { LIVE_NAMESPACE, expect, navigateTo, test } from './helpers'

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

/**
 * Comparisons (`ApiServerAuthRead`) and refusals the bridge sent back, recorded off the page's
 * own socket. Install before `app.open()`.
 */
async function tapApiServerAuth(page: Page): Promise<{
  comparisons: ApiServerAuthComparison[]
  errors: string[]
}> {
  const out = { comparisons: [] as ApiServerAuthComparison[], errors: [] as string[] }
  await page.routeWebSocket(/^ws:\/\/127\.0\.0\.1:\d+\//, (ws: WebSocketRoute) => {
    const server = ws.connectToServer()
    ws.onMessage((raw) => server.send(raw))
    server.onMessage((raw) => {
      const msg = JSON.parse(String(raw)) as DevBridgeServerMsg
      if (msg.op === 'error') out.errors.push(JSON.stringify(msg.error))
      if (msg.op === 'batch') {
        for (const env of msg.batch.envelopes) {
          const ev = env.event
          if (ev.type === 'ResourceRead' && ev.data.type === 'ApiServerAuthRead') {
            out.comparisons.push(ev.data.data)
          }
        }
      }
      ws.send(raw)
    })
  })
  return out
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

  /**
   * 045 T064/FR-024, live: the guided path's API server step and the one cluster write it can
   * offer, against the real backend as the namespace-scoped identity.
   *
   * Both commands are raw-sent through `window.__fathomDevBridge` (the convention
   * tests/live/actions.spec.ts uses to prove the write guard) rather than driven through the
   * guided path's UI: reaching the comparison step needs a reachable OIDC provider to answer a
   * discovery probe, and there is no IdP in a restricted walk.
   *
   * What FR-024 asks for -- the write showing up in the mutation audit log -- cannot be asserted
   * from any devbridge-backed suite: `WriteApiServerAuth` is `MutationTarget::ClusterScoped`, and
   * every bridge configuration refuses a cluster-scoped write before the backend ever dispatches
   * it (`tools/devbridge/src/guard.rs`), so nothing is there to audit. That refusal is what is
   * asserted here instead, which is the stronger half of the guarantee: no write can reach a
   * control plane through this transport at all, audited or not. The audited dispatch itself is
   * covered where it is reachable -- `BackendCmdV1::audit_summary` in src/protocol/v1/mutation.rs,
   * and the UI's own "audit log" wording in tests/e2e/guided-oidc.spec.ts.
   */
  test('the API server read answers live, and the write is refused before it can go unaudited', async ({
    app,
    page,
  }) => {
    const tap = await tapApiServerAuth(page)
    const pageErrors: string[] = []
    page.on('pageerror', (e) => pageErrors.push(e.message))
    await app.open()

    // 1. The read, carrying the draft being configured (FR-025). Read-only, so the bridge
    //    forwards it and the backend performs it as this identity.
    const accepted = await page.evaluate(async () => {
      const bridge = window.__fathomDevBridge
      if (!bridge) return 'no __fathomDevBridge on the page'
      const cmd: BackendCmd = {
        type: 'ReadApiServerAuth',
        data: {
          draft: {
            id: 'restricted-walk',
            issuer_url: 'https://idp.invalid/realms/fathom',
            client_id: 'fathom',
            scopes: ['openid', 'profile', 'email', 'groups'],
            ca_bundle_path: null,
            username_claim: 'preferred_username',
            groups_claim: 'groups',
            username_prefix: 'oidc:',
            groups_prefix: 'oidc:',
            admin_api_base: null,
          },
        },
      }
      try {
        await bridge.send(cmd)
        return 'ACCEPTED'
      } catch (err) {
        const e = err as { type?: string; data?: string }
        return `${e.type ?? '?'}: ${e.data ?? ''}`
      }
    })
    // Carrying a draft is not a write: the read-only bridge still forwards it (FR-025).
    expect(accepted).toBe('ACCEPTED')

    await expect.poll(() => tap.comparisons.length, { timeout: 30_000 }).toBeGreaterThan(0)
    const comparison = tap.comparisons[tap.comparisons.length - 1]
    if (!comparison) throw new Error('no ApiServerAuthRead')
    test.info().annotations.push({
      type: 'ApiServerAuthRead',
      description: JSON.stringify(comparison),
    })
    expect(comparison.read.context).toBe(liveContext())
    // FR-023/FR-010: a namespace-scoped identity cannot list kube-system pods, and a denied read
    // is a stated unknown in the API server's own words -- never an assumed pass, never an error.
    expect(comparison.read.checked).toBe(false)
    expect(comparison.read.reason ?? '').not.toBe('')
    // An unchecked read claims no differences either way, draft or no draft.
    expect(comparison.differences).toEqual([])
    // FR-022/FR-036: ownership comes from the provenance record, so a context Fathom did not
    // provision is not writable and is told why, naming itself.
    expect(comparison.writable).toBe(false)
    expect(comparison.not_writable_reason ?? '').toContain(liveContext())

    // 2. The write, raw. Cluster-scoped, so the guard refuses it before the backend sees it.
    const refused = await page.evaluate(async (cluster) => {
      const bridge = window.__fathomDevBridge
      if (!bridge) return 'no __fathomDevBridge on the page'
      const cmd: BackendCmd = {
        type: 'WriteApiServerAuth',
        data: {
          cluster,
          style: 'Legacy',
          issuer_url: 'https://idp.invalid/realms/fathom',
          client_id: 'fathom',
          username_claim: 'preferred_username',
          groups_claim: 'groups',
          username_prefix: 'oidc:',
          groups_prefix: 'oidc:',
        },
      }
      try {
        await bridge.send(cmd)
        return 'ACCEPTED'
      } catch (err) {
        const e = err as { type?: string; data?: string }
        return `${e.type ?? '?'}: ${e.data ?? ''}`
      }
    }, liveContext())
    expect(refused).toMatch(/^Forbidden: /)
    expect(refused).toContain('WriteApiServerAuth')
    expect(refused).toContain('cluster-scoped')

    // 3. The place the write would be recorded still works for this identity, and the pane says
    //    what it is: Fathom's own mutation log, not the API server's.
    await navigateTo(page, 'Audit Log', 'Team')
    await expect(page.getByTestId('audit-log-note')).toContainText('not Kubernetes')
    // A refused command never reached the backend, so there is nothing to have audited.
    await expect(page.getByTestId('audit-entry').filter({ hasText: 'APIServer' })).toHaveCount(0)
    expect(pageErrors).toEqual([])
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
