/**
 * US4: logs, shells and stats in tabs, side by side (T064), against the fake backend. Log lines are
 * untrusted text; the layout comes back after a reload; streams survive the proxy's idle cut; the
 * shell is absent when the server turns it off; an expired session offers new shells rather than
 * starting them unasked.
 */
import type { Page } from '@playwright/test'

import { CloseCode } from '../../../src/protocol/generated/constants'
import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const T = (s: number) => `2026-10-23T12:00:${String(s).padStart(2, '0')}Z`

function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList([f.stack('web')])
  backend.logs['web-app-1'] = [`${T(1)} booting`, `${T(2)} \u001b[32mready\u001b[0m on :8080`, `${T(3)} GET /health 200`]
  backend.logs.web = ['app | ' + `${T(1)} stack line`]
}

async function openStack(page: Page): Promise<void> {
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
}

const container = (page: Page) => page.locator('[data-container="web-app-1"]')
const tab = (page: Page, kind: string) => page.locator(`[data-tab-kind="${kind}"]`)
// The stack page has a log console of its own (one more stream, the first running container's);
// these tests are about the tabs, so they look inside the tab dock.
const dock = (page: Page) => page.getByTestId('tab-dock')
const logView = (page: Page) => dock(page).getByTestId('log-view')

test.describe('tabs', () => {
  test.beforeEach(({ backend }) => seed(backend))

  test('a log tab shows colour as styled text, other escapes removed and markup kept literal', async ({ page, backend }) => {
    backend.logs['web-app-1']?.push(`${T(4)} <img src=x onerror="window.__pwned=1"> <b>bold</b>`)
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()

    const open = await backend.waitForCmd('OpenLogs')
    expect(open.data).toEqual({ target: { kind: 'container', container: 'web-app-1' }, tail: 1000 })
    await expect(dock(page).locator('[data-field="phase"]')).toHaveAttribute('data-phase', 'live')
    await expect(logView(page)).toContainText('ready on :8080')
    await expect(logView(page)).not.toContainText('\u001b')
    await expect(logView(page)).not.toContainText('[32m')
    // SGR green is a styled span around the text it coloured, in a theme colour.
    const green = logView(page).locator('[data-ansi][data-fg="2"]')
    await expect(green).toHaveText('ready')
    expect(await green.evaluate((el) => getComputedStyle(el).color)).not.toBe(
      await logView(page).evaluate((el) => getComputedStyle(el).color),
    )
    await expect(logView(page)).toContainText('<img src=x onerror="window.__pwned=1"> <b>bold</b>')
    await expect(logView(page).locator('img, b')).toHaveCount(0)
    expect(await page.evaluate(() => (window as unknown as { __pwned?: number }).__pwned)).toBeUndefined()

    // Live lines append.
    backend.pushLogs([`${T(5)} GET / 200`])
    await expect(logView(page)).toContainText('GET / 200')
  })

  test('the same logs open once; search narrows and counts', async ({ page, backend }) => {
    await openStack(page)
    const btn = container(page).getByRole('button', { name: 'Logs for web-app-1' })
    await btn.click()
    await expect(logView(page)).toContainText('booting')
    await btn.click()
    await expect(tab(page, 'logs')).toHaveCount(1)
    // The console's stream and the tab's: the second click opened nothing.
    expect(backend.received.filter((c) => c.type === 'OpenLogs')).toHaveLength(2)

    await dock(page).getByRole('searchbox', { name: 'Search logs' }).fill('health')
    await expect(dock(page).locator('[data-field="matches"]')).toHaveText('1 of 3')
    await expect(logView(page)).toContainText('GET /health 200')
    await expect(logView(page)).not.toContainText('booting')
    await dock(page).getByRole('searchbox', { name: 'Search logs' }).fill('nothing-matches')
    await expect(dock(page).locator('[data-pane-state="no-match"]')).toBeVisible()
  })

  test('a search result shows the whole log at that line, flashed, with follow paused', async ({ page, backend }) => {
    backend.logs['web-app-1'] = Array.from({ length: 400 }, (_, i) =>
      i === 120 ? `${T(1)} {"level":"error","msg":"needle in the haystack"}` : `${T(1)} line ${String(i)}`,
    )
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(logView(page)).toContainText('line 399')
    await expect(dock(page).getByRole('button', { name: 'Follow' })).toHaveAttribute('aria-pressed', 'true')

    await dock(page).getByRole('searchbox', { name: 'Search logs' }).fill('needle')
    await expect(dock(page).locator('[data-field="matches"]')).toHaveText('1 of 400')
    await expect(logView(page).locator('mark')).toHaveText('needle')
    await expect(dock(page).getByRole('button', { name: 'Level error, 1 lines' })).toBeVisible()
    await logView(page).locator('[data-action="show-in-context"]').click()

    await expect(logView(page)).toHaveAttribute('data-view', 'context')
    const line = logView(page).locator('[data-line="120"]')
    await expect(line).toHaveAttribute('data-flash', '')
    await expect(line).toBeInViewport()
    await expect(line).toBeFocused()
    await expect(logView(page)).toContainText('line 119')
    await expect(dock(page).getByRole('button', { name: 'Follow' })).toHaveAttribute('aria-pressed', 'false')
    // Lines arriving now do not move the view.
    backend.pushLogs([`${T(9)} after the jump`])
    await expect(logView(page)).toContainText('after the jump')
    await expect(line).toBeInViewport()

    await dock(page).getByRole('button', { name: /Back to results/ }).click()
    await expect(logView(page)).toHaveAttribute('data-view', 'results')
    await expect(logView(page).locator('[data-line]')).toHaveCount(1)
  })

  test('pause freezes the view and counts what arrives; resume catches up', async ({ page, backend }) => {
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(logView(page)).toContainText('booting')
    await dock(page).getByRole('button', { name: 'Pause' }).click()
    backend.pushLogs([`${T(6)} after pause one`, `${T(7)} after pause two`])
    await expect(dock(page).locator('[data-field="new-lines"]')).toHaveText('2 new')
    await expect(logView(page)).not.toContainText('after pause')
    await dock(page).getByRole('button', { name: 'Resume' }).click()
    await expect(logView(page)).toContainText('after pause two')
  })

  test('logs, a shell and stats side by side in panes, restored after a reload', async ({ page, backend }) => {
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await container(page).getByRole('button', { name: 'Shell in web-app-1' }).click()
    await container(page).getByRole('button', { name: 'Stats for web-app-1' }).click()
    await expect(page.getByRole('tablist', { name: 'Tabs' }).getByRole('tab')).toHaveCount(3)

    // Stats: the sample for the container, by its full id.
    const watch = await backend.waitForCmd('WatchStats')
    expect(watch.data.targets).toEqual([{ kind: 'container', container: 'web-app-1' }])
    const row = page.locator('[data-stats-row="web-app-1"]')
    await expect(row.locator('[data-field="cpu"]')).toHaveText('1.5%')
    await expect(row.locator('[data-field="memory"]')).toHaveText('64 MiB / 512 MiB')

    // Split stats into a pane of its own, then the shell into a third.
    await page.getByRole('button', { name: 'Split: move tab to a new pane' }).first().click()
    await expect(page.locator('[data-pane]')).toHaveCount(2)
    await page.locator('[data-pane="pane-1"]').getByRole('tab', { name: /Shell/ }).click()
    await page.locator('[data-pane="pane-1"]').getByRole('button', { name: 'Split: move tab to a new pane' }).click()
    await expect(page.locator('[data-pane]')).toHaveCount(3)

    // All three on screen at once, each live.
    await expect(logView(page)).toContainText('booting')
    await expect(page.locator('[data-stats-row="web-app-1"] [data-field="cpu"]')).toHaveText('1.5%')
    const term = page.locator('[data-tab-body="shell"] [data-testid^="terminal-"]')
    await expect(term).toContainText('/ #')
    await term.click()
    await page.keyboard.type('echo hi')
    await expect(term).toContainText('echo hi')
    const input = await backend.waitForCmd('ShellInput')
    expect(Buffer.from(input.data.bytes_b64, 'base64').toString('utf8').length).toBeGreaterThan(0)

    // Reload: the same tabs in the same panes, the logs reopened from the tail.
    const opens = backend.received.filter((c) => c.type === 'OpenLogs').length
    await page.reload()
    await expect(page.locator('[data-pane]')).toHaveCount(3)
    await expect(page.locator('[data-pane="pane-1"] [data-tab-kind="logs"]')).toHaveCount(1)
    await expect(page.locator('[data-pane="pane-2"] [data-tab-kind="stats"]')).toHaveCount(1)
    await expect(page.locator('[data-pane="pane-3"] [data-tab-kind="shell"]')).toHaveCount(1)
    await expect(logView(page)).toContainText('booting')
    // The tab's stream and the console's.
    await expect.poll(() => backend.received.filter((c) => c.type === 'OpenLogs').length).toBe(opens + 2)
    // A shell cannot be resumed across a reload: it is a new session.
    await expect(page.locator('[data-tab-body="shell"] [data-testid^="terminal-"]')).toContainText('/ #')
  })

  test('closing a tab closes its stream', async ({ page, backend }) => {
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(logView(page)).toContainText('booting')
    expect(backend.openStreams()).toHaveLength(2)
    await page.getByRole('button', { name: 'Close logs web-app-1' }).click()
    const close = await backend.waitForCmd('CloseStream')
    expect(backend.openStreams()).not.toContain(close.data.stream_id)
    await expect(page.getByTestId('tab-dock')).toHaveCount(0)
  })

  test('tabs are arranged from the keyboard', async ({ page }) => {
    await openStack(page)
    await page.getByRole('button', { name: 'Logs for stack web' }).click()
    await page.getByRole('button', { name: 'Stats for stack web' }).click()
    await expect(page.getByRole('tablist', { name: 'Tabs' }).getByRole('tab')).toHaveText([/web/, /web/])
    const first = page.locator('[data-tab-id]').first()
    await expect(first).toHaveAttribute('data-tab-kind', 'logs')
    await page.getByRole('button', { name: 'Move tab left' }).focus()
    await page.keyboard.press('Enter')
    await expect(page.locator('[data-tab-id]').first()).toHaveAttribute('data-tab-kind', 'stats')
    await page.getByRole('tab', { name: /Logs/ }).focus()
    await page.keyboard.press('Enter')
    await expect(page.getByRole('tab', { name: /Logs/ })).toHaveAttribute('aria-selected', 'true')
  })

  test('a log stream cut by the proxy reopens since the last line, without repeating it', async ({ page, backend }) => {
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(logView(page)).toContainText('GET /health 200')
    // The console's open, then the tab's: both reopen the same way.
    await backend.waitForCmd('OpenLogs')
    await backend.waitForCmd('OpenLogs')

    backend.closeStreams({ kind: 'disconnected' })
    const again = await backend.waitForCmd('OpenLogs')
    expect(again.data).toEqual({ target: { kind: 'container', container: 'web-app-1' }, tail: 1000, since: T(3) })
    await expect(dock(page).locator('[data-notice]')).toHaveText('Reconnected')
    await expect(dock(page).locator('[data-field="phase"]')).toHaveAttribute('data-phase', 'live')
    // The fake replays the whole tail; the tab keeps one copy of each line.
    await expect(logView(page).getByText('GET /health 200')).toHaveCount(1)
  })

  test('an ended stream says when and why, and reopens on request', async ({ page, backend }) => {
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(logView(page)).toContainText('booting')
    backend.closeStreams({ kind: 'ended' })
    const ended = dock(page).locator('[data-ended]')
    await expect(ended).toContainText('The container stopped.')
    await expect(ended).toContainText(/Ended \d/)
    await ended.getByRole('button', { name: 'Reopen' }).click()
    await expect(dock(page).locator('[data-field="phase"]')).toHaveAttribute('data-phase', 'live')
    await expect(ended).toHaveCount(0)
  })

  test('a log the server refuses says so', async ({ page, backend }) => {
    backend.deny('OpenLogs')
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(dock(page).locator('[data-pane-state="not-permitted"]')).toBeVisible()
  })

  test('a dropped socket reopens logs and ends shells, offering a new one', async ({ page, backend }) => {
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await container(page).getByRole('button', { name: 'Shell in web-app-1' }).click()
    await page.getByRole('button', { name: 'Split: move tab to a new pane' }).first().click()
    await expect(page.locator('[data-tab-body="shell"] [data-testid^="terminal-"]')).toContainText('/ #')
    const shells = backend.received.filter((c) => c.type === 'OpenShell').length

    backend.closeSockets(1011)
    await expect(page.locator('[data-ended] [data-field="end-reason"]')).toContainText('The connection to Depths dropped.')
    await expect(dock(page).locator('[data-notice]')).toHaveText('Reconnected')
    expect(backend.received.filter((c) => c.type === 'OpenShell')).toHaveLength(shells)

    await page.getByRole('button', { name: 'New session' }).click()
    await expect.poll(() => backend.received.filter((c) => c.type === 'OpenShell').length).toBe(shells + 1)
    await expect(page.locator('[data-tab-body="shell"]')).toHaveAttribute('data-shell-status', 'active')
  })
})

test.describe('shell turned off', () => {
  test('no shell button, and a stored shell tab is dropped', async ({ page, backend }) => {
    seed(backend)
    await openStack(page)
    await container(page).getByRole('button', { name: 'Shell in web-app-1' }).click()
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(page.getByRole('tablist', { name: 'Tabs' }).getByRole('tab')).toHaveCount(2)

    backend.me = f.me({ features: { shell: false, edit: true, host_metrics: false } })
    await page.reload()
    await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
    await expect(container(page).getByRole('button', { name: 'Shell in web-app-1' })).toHaveCount(0)
    await expect(page.getByRole('tablist', { name: 'Tabs' }).getByRole('tab')).toHaveCount(1)
    await expect(tab(page, 'shell')).toHaveCount(0)
  })
})

test.describe('session expiry', () => {
  test('signs out with a notice, and after signing in the tabs return with shells offered, not started', async ({ page, backend }) => {
    seed(backend)
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await container(page).getByRole('button', { name: 'Shell in web-app-1' }).click()
    await expect(page.locator('[data-tab-body="shell"] [data-testid^="terminal-"]')).toContainText('/ #')

    const me = backend.me
    backend.me = null
    backend.closeSockets(CloseCode.Unauthenticated)
    await expect(page.locator('[data-page="sign-in"]')).toBeVisible()
    await expect(page.locator('[data-notice="expired"]')).toContainText('Your session expired')

    backend.me = me
    const shells = backend.received.filter((c) => c.type === 'OpenShell').length
    await page.goto('/stacks/web')
    await expect(page.getByRole('tablist', { name: 'Tabs' }).getByRole('tab')).toHaveCount(2)
    await page.getByRole('tab', { name: /Shell/ }).click()
    await expect(page.locator('[data-field="end-reason"]')).toContainText('Your session expired.')
    await expect(page.getByRole('button', { name: 'New session' })).toBeVisible()
    expect(backend.received.filter((c) => c.type === 'OpenShell')).toHaveLength(shells)
    // The log tab reopened on its own.
    await page.getByRole('tab', { name: /Logs/ }).click()
    await expect(logView(page)).toContainText('booting')
  })
})

test.describe('narrow screens', () => {
  test.use({ viewport: { width: 390, height: 800 } })

  test('one strip holds every tab, and the top bar switches between the page and the tabs', async ({ page, backend }) => {
    seed(backend)
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(page.locator('[data-dock="narrow"]')).toBeVisible()
    await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeHidden()
    await expect(page.getByRole('button', { name: 'Split: move tab to a new pane' })).toHaveCount(0)

    await page.getByTestId('tabs-toggle').click()
    await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
    await container(page).getByRole('button', { name: 'Stats for web-app-1' }).click()
    await expect(page.getByRole('tablist', { name: 'Tabs' }).getByRole('tab')).toHaveCount(2)
    await expect(page.getByRole('tab', { name: /Stats/ })).toHaveAttribute('aria-selected', 'true')
    // No sideways scroll at phone width.
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  })
})

test.describe('six tabs, resized', () => {
  test('six tabs across kinds, split and resized, come back in order with their sizes', async ({ page, backend }) => {
    seed(backend)
    await openStack(page)
    await page.getByRole('button', { name: 'Logs for stack web' }).click()
    await page.getByRole('button', { name: 'Stats for stack web' }).click()
    await page.getByRole('button', { name: 'Logs for service app' }).click()
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await container(page).getByRole('button', { name: 'Shell in web-app-1' }).click()
    await container(page).getByRole('button', { name: 'Stats for web-app-1' }).click()
    await expect(page.getByRole('tablist', { name: 'Tabs' }).getByRole('tab')).toHaveCount(6)

    await page.getByRole('button', { name: 'Split: move tab to a new pane' }).click()
    await expect(page.locator('[data-pane]')).toHaveCount(2)
    const width = async () => (await page.locator('[data-pane="pane-1"]').boundingBox())?.width ?? 0
    const before = await width()

    // The divider is a keyboard-operable separator.
    await page.getByRole('separator', { name: 'Resize panes 1 and 2' }).focus()
    for (let i = 0; i < 5; i++) await page.keyboard.press('ArrowRight')
    await expect.poll(width).toBeGreaterThan(before + 20)
    const widened = await width()

    const kinds = () => page.locator('[data-tab-id]').evaluateAll((els) => els.map((e) => e.getAttribute('data-tab-kind')))
    const order = await kinds()
    await page.reload()
    await expect(page.getByRole('tablist', { name: 'Tabs' }).getByRole('tab')).toHaveCount(6)
    expect(await kinds()).toEqual(order)
    await expect(page.locator('[data-pane]')).toHaveCount(2)
    await expect.poll(async () => Math.abs((await width()) - widened)).toBeLessThan(8)
  })
})

test.describe('shell', () => {
  test('colour, resize and Ctrl-C reach the shell, and the end says when and why', async ({ page, backend }) => {
    seed(backend)
    backend.shellBanner = '\u001b[31mred prompt\u001b[0m / # '
    await openStack(page)
    await container(page).getByRole('button', { name: 'Shell in web-app-1' }).click()
    const open = await backend.waitForCmd('OpenShell')
    expect(open.data.container).toBe('web-app-1')
    const term = page.locator('[data-tab-body="shell"] [data-testid^="terminal-"]')
    await expect(term).toContainText('red prompt')
    // Rendered in colour, not as escape text.
    await expect(term).not.toContainText('[31m')
    await expect(term.locator('[class*="xterm-fg-1"]').first()).toBeAttached()

    await term.click()
    await page.keyboard.press('Control+C')
    await expect
      .poll(() =>
        backend.received.some((c) => c.type === 'ShellInput' && Buffer.from(c.data.bytes_b64, 'base64').toString('latin1') === '\u0003'),
      )
      .toBe(true)

    const resizes = () => backend.received.filter((c) => c.type === 'ShellResize')
    const count = resizes().length
    await page.setViewportSize({ width: 900, height: 500 })
    await expect.poll(() => resizes().length).toBeGreaterThan(count)
    const last = resizes().at(-1)
    expect(last?.type === 'ShellResize' && last.data.cols > 2 && last.data.rows > 2).toBe(true)

    backend.closeStreams({ kind: 'ended' })
    const reason = page.locator('[data-field="end-reason"]')
    await expect(reason).toContainText(/Session ended \d/)
    await expect(reason).toContainText('The shell exited.')
    await expect(page.getByRole('button', { name: 'New session' })).toBeVisible()
  })
})

test.describe('logs, in depth', () => {
  test('follow keeps the newest line in view until scrolled away', async ({ page, backend }) => {
    seed(backend)
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(logView(page)).toContainText('booting')
    backend.pushLogs(Array.from({ length: 400 }, (_, i) => `${T(10)} line ${String(i)}`))
    await expect(logView(page)).toContainText('line 399')
    const atBottom = () => logView(page).evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight < 40)
    await expect.poll(atBottom).toBe(true)
    const follow = dock(page).getByRole('button', { name: 'Follow' })
    await expect(follow).toHaveAttribute('aria-pressed', 'true')

    await logView(page).evaluate((el) => {
      el.scrollTop = 0
      el.dispatchEvent(new Event('scroll'))
    })
    await expect(follow).toHaveAttribute('aria-pressed', 'false')
    backend.pushLogs([`${T(11)} later`])
    await expect.poll(atBottom).toBe(false)
    await follow.click()
    await expect.poll(atBottom).toBe(true)
  })

  test('the time window keeps lines between From and To', async ({ page, backend }) => {
    seed(backend)
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(logView(page)).toContainText('booting')
    // The inputs take local time, as the browser shows it.
    const local = (iso: string) =>
      page.evaluate((v) => {
        const d = new Date(v)
        const p = (n: number) => String(n).padStart(2, '0')
        return `${String(d.getFullYear())}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
      }, iso)
    await dock(page).getByLabel('From time').fill(await local(T(2)))
    await expect(dock(page).locator('[data-field="matches"]')).toHaveText('2 of 3')
    await expect(logView(page)).not.toContainText('booting')
    await dock(page).getByLabel('To time').fill(await local(T(2)))
    await expect(dock(page).locator('[data-field="matches"]')).toHaveText('1 of 3')
    await expect(logView(page)).toContainText('ready on :8080')
  })

  test('past the ring cap the oldest lines go, and the view says so', async ({ page, backend }) => {
    seed(backend)
    await openStack(page)
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await expect(logView(page)).toContainText('booting')
    const marker = dock(page).locator('[data-pane-state="logs-truncated"]')
    await expect(marker).toHaveCount(0)
    for (let b = 0; b < 4; b++) backend.pushLogs(Array.from({ length: 2600 }, (_, i) => `bulk ${String(b)}-${String(i)}`))
    await expect(marker).toBeAttached()
    await expect(marker).toContainText(/\d/)
  })
})

test.describe('stats', () => {
  test('a stats tab samples only while it is on screen', async ({ page, backend }) => {
    seed(backend)
    const watches = () => backend.received.filter((c) => c.type === 'WatchStats').length
    const unwatches = () => backend.received.filter((c) => c.type === 'UnwatchStats').length
    await openStack(page)
    await container(page).getByRole('button', { name: 'Stats for web-app-1' }).click()
    await backend.waitForCmd('WatchStats')
    // The logs tab takes the pane: the stats tab is hidden, so the watch goes.
    await container(page).getByRole('button', { name: 'Logs for web-app-1' }).click()
    await backend.waitForCmd('UnwatchStats')
    const w = watches()
    await expect(logView(page)).toContainText('booting')
    expect(watches()).toBe(w)

    await page.getByRole('tab', { name: /Stats/ }).click()
    await expect.poll(watches).toBe(w + 1)

    // A hidden page stops watching too.
    const u = unwatches()
    await page.evaluate(() => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' })
      document.dispatchEvent(new Event('visibilitychange'))
    })
    await expect.poll(unwatches).toBe(u + 1)
  })
})
