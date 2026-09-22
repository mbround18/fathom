/**
 * Promo capture helpers: a read-only `app` fixture, scene settling, redaction, and shot output.
 *
 * Read-only: the page's bridge socket is routed through Playwright, and any write command
 * (delete, scale, restart, …) is dropped instead of forwarded. `expectNoWrites` fails the scene
 * if one was attempted, so a promo run can never change the cluster.
 */
import { mkdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import type { Page, WebSocketRoute } from '@playwright/test'

import type { DevBridgeClientMsg } from '../../src/protocol/types'
import { expect, listPods, test as live } from '../live/helpers'

export { expect, kubectlJson, LIVE_NAMESPACE, listPods } from '../live/helpers'
export type { KubeDeployment, KubePod } from '../live/helpers'
export { liveContext } from '../live/env'

export const OUT_DIR = fileURLToPath(new URL('../../promo-out/', import.meta.url))
export const SHOTS_DIR = `${OUT_DIR}shots/`

const WRITE_CMD =
  /^(Delete|BulkDelete|Scale|Restart|Rollback|Cordon|Uncordon|Drain|Evict|Apply|Create|Patch|Update|Edit|Suspend|Resume|Trigger)/

/** Route the bridge socket, dropping writes. Install before `app.open()`. */
async function readOnlyBridge(page: Page, blocked: string[]): Promise<void> {
  await page.routeWebSocket(/^ws:\/\/127\.0\.0\.1:\d+\//, (ws: WebSocketRoute) => {
    const server = ws.connectToServer()
    ws.onMessage((raw) => {
      const msg = JSON.parse(String(raw)) as DevBridgeClientMsg
      if (msg.op === 'send' && WRITE_CMD.test(msg.cmd.type)) {
        blocked.push(msg.cmd.type)
        return
      }
      server.send(raw)
    })
    server.onMessage((raw) => {
      ws.send(raw)
    })
  })
}

/** `app` (the live fixture) behind a read-only bridge; fails the scene if a write was tried. */
export const test = live.extend<{ blockedWrites: string[] }>({
  blockedWrites: [
    async ({ page }, provide) => {
      const blocked: string[] = []
      await readOnlyBridge(page, blocked)
      await provide(blocked)
      expect(blocked, 'promo capture must never send a write command').toEqual([])
    },
    { auto: true },
  ],
})

/**
 * Node names are hosting identifiers; promo material shows `node-1`, `node-2`, … instead.
 * Set FATHOM_PROMO_REDACT=0 to keep the real names.
 */
export function redactions(): Array<[string, string]> {
  if (process.env.FATHOM_PROMO_REDACT === '0') return []
  const nodes = [...new Set(listPods().map((p) => p.spec?.nodeName).filter(Boolean))] as string[]
  return nodes.sort().map((n, i) => [n, `node-${String(i + 1)}`])
}

/** Rewrite visible text (not inputs the app reads) so redacted names never reach a pixel. */
async function redact(page: Page, pairs: Array<[string, string]>): Promise<void> {
  if (pairs.length === 0) return
  await page.evaluate((replacements) => {
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      let text = n.nodeValue ?? ''
      for (const [from, to] of replacements) text = text.split(from).join(to)
      if (text !== n.nodeValue) n.nodeValue = text
    }
  }, pairs)
  const leaked = await page.evaluate(
    (names) => names.filter((name) => document.body.innerText.includes(name)),
    pairs.map(([from]) => from),
  )
  expect(leaked, 'redacted names still visible').toEqual([])
}

/** Fonts loaded, no loading panes, no hover, then a PNG in promo-out/shots/. */
export async function shoot(
  page: Page,
  name: string,
  pairs: Array<[string, string]>,
): Promise<void> {
  await page.evaluate(() => document.fonts.ready)
  await expect(page.locator('[data-pane-state="loading"]')).toHaveCount(0)
  await expect(page.locator('[data-testid="action-toast"]')).toHaveCount(0)
  await page.mouse.move(0, 0)
  // Virtualised rows and Radix popovers settle within a frame or two.
  await page.waitForTimeout(400)
  await redact(page, pairs)
  mkdirSync(SHOTS_DIR, { recursive: true })
  await page.screenshot({
    path: `${SHOTS_DIR}${name}.png`,
    animations: 'disabled',
    caret: 'hide',
  })
}
