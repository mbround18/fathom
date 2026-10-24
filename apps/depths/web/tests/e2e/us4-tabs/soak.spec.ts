/**
 * US4 soak (T065, SC-005): six log tabs streaming 50 lines/s each for a simulated hour, time
 * compressed (each push is five seconds of lines, sent back to back). The JS heap stays bounded,
 * since each tab keeps a ring of lines, and typing into a shell while the lines pour in stays under
 * 100 ms from key to paint.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const STACKS = ['web', 'api', 'db'] as const
const LINES_PER_SECOND = 50
const CHUNK_SECONDS = 5
const CHUNKS_PER_MINUTE = 60 / CHUNK_SECONDS
const MINUTES = 60
const START = Date.parse('2026-10-23T12:00:00Z')

function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList(STACKS.map((s) => f.stack(s)))
}

/** Chunk `n` of the hour: five seconds at 50 lines/s, timestamped 20 ms apart. */
function chunk(n: number): string[] {
  const base = START + n * CHUNK_SECONDS * 1000
  return Array.from({ length: CHUNK_SECONDS * LINES_PER_SECOND }, (_, i) => {
    const at = new Date(base + i * (1000 / LINES_PER_SECOND)).toISOString()
    return `${at} \u001b[36minfo\u001b[0m request ${String(n)}-${String(i)} served in 3ms`
  })
}

async function heapMb(page: Page): Promise<number> {
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('HeapProfiler.collectGarbage')
  const { usedSize } = await cdp.send('Runtime.getHeapUsage')
  await cdp.detach()
  return usedSize / (1024 * 1024)
}

const frame = (page: Page) => page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => r())))

test('six streaming tabs for an hour keep the heap bounded and input under 100 ms', async ({ page, backend }) => {
  test.setTimeout(240_000)
  seed(backend)
  await page.goto('/')

  // Six log tabs: each stack's combined logs and its container's, then a shell to type into.
  // Navigated in the app, not by reloading, so the streams opened so far stay open.
  for (const s of STACKS) {
    await page.locator(`[data-stack="${s}"]`).getByRole('link', { name: s }).click()
    await expect(page.getByRole('heading', { name: s, level: 1 })).toBeVisible()
    await page.getByRole('button', { name: `Logs for stack ${s}` }).click()
    await page.locator(`[data-container="${s}-app-1"]`).getByRole('button', { name: `Logs for ${s}-app-1` }).click()
    if (s !== 'db') await page.getByRole('link', { name: 'Stacks' }).first().click()
  }
  await page.locator('[data-container="db-app-1"]').getByRole('button', { name: 'Shell in db-app-1' }).click()
  await expect(page.getByRole('tablist', { name: 'Tabs' }).getByRole('tab')).toHaveCount(7)
  // The seven tabs, and the console of the stack on screen (each earlier page closed its own).
  await expect.poll(() => backend.openStreams().length).toBe(8)

  // Three panes, so several streams render at once: shell | logs | logs.
  await page.getByRole('button', { name: 'Split: move tab to a new pane' }).click()
  await page.locator('[data-pane="pane-1"]').getByRole('tab').first().click()
  await page.locator('[data-pane="pane-1"]').getByRole('button', { name: 'Split: move tab to a new pane' }).click()
  await expect(page.locator('[data-pane]')).toHaveCount(3)
  const term = page.locator('[data-tab-body="shell"] [data-testid^="terminal-"]')
  await expect(term).toContainText('/ #')

  // Key-to-paint latency, measured in the page from the event's own timestamp.
  await page.evaluate(() => {
    const w = window as unknown as { __lat: number[] }
    w.__lat = []
    document.addEventListener(
      'keydown',
      (e) => {
        const t0 = e.timeStamp
        requestAnimationFrame(() => setTimeout(() => w.__lat.push(performance.now() - t0), 0))
      },
      { capture: true },
    )
  })

  let baseline = 0
  await term.click()
  const last = MINUTES * CHUNKS_PER_MINUTE - 1
  for (let n = 0; n <= last; n++) {
    backend.pushLogs(chunk(n))
    // One key a simulated minute, typed while that chunk is arriving.
    if (n % CHUNKS_PER_MINUTE === 0) await page.keyboard.type('x')
    await frame(page)
    // Ten minutes in, every ring is full: what the heap holds from here on is the steady state.
    if (n === 10 * CHUNKS_PER_MINUTE) baseline = await heapMb(page)
  }
  // Every line arrived: the visible tab holds its ring and says how many older ones it let go.
  const dropped = (last + 1) * CHUNK_SECONDS * LINES_PER_SECOND - 10_000
  await expect(page.locator('[data-tab-body="logs"]').first()).toContainText(`${String(dropped)} older lines dropped`)
  await expect(page.locator('[data-pane-state="logs-truncated"]').first()).toBeAttached()

  const end = await heapMb(page)
  const lat = await page.evaluate(() => (window as unknown as { __lat: number[] }).__lat)
  const sorted = [...lat].sort((a, b) => a - b)
  const p95 = sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.95))] ?? Infinity
  console.log(
    `soak: heap ${baseline.toFixed(1)} MiB at 10 min, ${end.toFixed(1)} MiB at ${String(MINUTES)} min; ` +
      `key-to-paint p95 ${p95.toFixed(1)} ms over ${String(lat.length)} keys`,
  )

  expect(lat.length).toBeGreaterThanOrEqual(MINUTES)
  // Fifty more minutes of lines, and no more than a little more heap.
  expect(end).toBeLessThan(baseline + 24)
  expect(end).toBeLessThan(256)
  expect(p95).toBeLessThan(100)
})
