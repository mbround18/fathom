/**
 * The overview's metrics: host figures for admins (`WatchHost`, only when the server reads the
 * host's `/proc`), and CPU and memory per stack for everyone, busiest first.
 */
import { expect, fixtures as f, sessions, test } from '../../support/fake-backend'

const hostOn = { shell: true, edit: true, host_metrics: true }

function twoStacks() {
  const web = f.stack('web')
  const db = f.stack('db')
  const id = (s: typeof web) => s.services[0]?.containers[0]?.id ?? ''
  return { list: f.stackList([web, db]), webId: id(web), dbId: id(db) }
}

test('an admin sees the host panel, and it stops watching when they leave the overview', async ({ page, backend }) => {
  backend.me = sessions.admin({ features: hostOn })
  backend.stacks = twoStacks().list
  await page.goto('/')
  await backend.waitForCmd('WatchHost')

  const host = page.locator('[data-section="host"]')
  await expect(host.locator('[data-host="cpu"] [data-field="value"]')).toHaveText('25.0%')
  await expect(host.locator('[data-host="memory"] [data-field="detail"]')).toHaveText('2.0 GiB of 8.0 GiB · swap 0 B')
  await expect(host.locator('[data-host="disk"] [data-field="detail"]')).toHaveText('60 GiB free of 100 GiB')
  await expect(host.locator('[data-host="load"] [data-field="value"]')).toHaveText('1.00')
  await expect(host.getByRole('meter', { name: 'Memory' })).toHaveAttribute('aria-valuenow', '25')
  await expect(host.locator('[data-field="host-summary"]')).toHaveText('up 1d 0h · 3 running · 1 stopped')

  // Live ticks extend the sparkline.
  backend.push({ type: 'HostSample', data: f.hostSample({ cpu_percent: 80 }) })
  await expect(host.locator('[data-host="cpu"] [data-field="value"]')).toHaveText('80.0%')
  await expect(host.getByRole('img', { name: 'CPU history, last 2 samples' })).toBeVisible()

  await page.locator('[data-stack-usage="web"] a').click()
  await expect(page).toHaveURL(/\/stacks\/web$/)
  await backend.waitForCmd('UnwatchHost')
})

test('the host panel is hidden from users and when host metrics are off', async ({ page, backend }) => {
  backend.stacks = twoStacks().list
  backend.me = sessions.admin()
  await page.goto('/')
  await expect(page.locator('[data-page="overview"]')).toBeVisible()
  await expect(page.locator('[data-section="host"]')).toHaveCount(0)

  backend.me = sessions.user(['web'], { features: hostOn })
  await page.reload()
  await expect(page.locator('[data-page="overview"]')).toBeVisible()
  await expect(page.locator('[data-section="host"]')).toHaveCount(0)
  expect(backend.received.some((c) => c.type === 'WatchHost')).toBe(false)
})

test('stacks are listed by CPU, summed over their containers, and link to the stack', async ({ page, backend }) => {
  const { list, webId, dbId } = twoStacks()
  backend.stacks = list
  backend.samples = [
    f.sample(webId, { cpu_percent: 2, memory_bytes: 64 * 1024 * 1024 }),
    f.sample(dbId, { cpu_percent: 30.25, memory_bytes: 1536 * 1024 * 1024 }),
  ]
  await page.goto('/')
  const watch = await backend.waitForCmd('WatchStats')
  expect(watch.data.targets).toEqual([
    { kind: 'stack', stack: 'db' },
    { kind: 'stack', stack: 'web' },
  ])

  const rows = page.locator('[data-stack-usage]')
  await expect(rows).toHaveCount(2)
  await expect(rows.nth(0)).toHaveAttribute('data-stack-usage', 'db')
  await expect(rows.nth(0).locator('[data-field="cpu"]')).toHaveText('30.3%')
  await expect(rows.nth(0).locator('[data-field="memory"]')).toHaveText('1.5 GiB')
  await expect(rows.nth(1).locator('[data-field="cpu"]')).toHaveText('2.0%')

  await rows.nth(0).getByRole('link', { name: 'db' }).click()
  await expect(page).toHaveURL(/\/stacks\/db$/)
  await backend.waitForCmd('UnwatchStats')
})

test('on a phone an admin gets a one-line host summary above the list', async ({ page, backend }) => {
  await page.setViewportSize({ width: 390, height: 800 })
  backend.me = sessions.admin({ features: hostOn })
  backend.stacks = twoStacks().list
  await page.goto('/')
  const strip = page.locator('[data-section="host-strip"]')
  await expect(strip.locator('[data-host="cpu"]')).toHaveText('CPU 25.0%')
  await expect(strip.locator('[data-host="memory"]')).toHaveText('Mem 25.0%')
  await expect(page.locator('[data-stack="web"]')).toBeVisible()
  const width = await page.evaluate(() => document.documentElement.scrollWidth)
  expect(width).toBeLessThanOrEqual(390)
})
