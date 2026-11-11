/**
 * Spec 066 T054: the Repository page (each tab, with a selection open) and the Add host dialog
 * (form and result), at a phone width (360 px) and a desktop width (1280 px), in the dark and the
 * light theme, fit the viewport: neither the page nor the dialog scrolls sideways. The helpers are
 * the ones tests/e2e/polish-screens.spec.ts uses. KEEL_SHOTS=<dir> also saves each screen.
 */
import type { Page } from '@playwright/test'

import { AGE } from '../../support/catalog-fixtures'
import { expect, test } from '../../support/fake-backend'
import { confirmChange, setUp } from './repo-setup'

async function widths(page: Page): Promise<{ doc: [number, number]; main: [number, number]; offenders: string[] }> {
  return page.evaluate(() => {
    const root = document.documentElement
    const main = document.querySelector('main')
    const edge = main ? main.getBoundingClientRect().right : root.clientWidth
    const offenders = [...(main ?? document.body).querySelectorAll('*')]
      .filter((el) => el.getBoundingClientRect().right > edge + 0.5)
      .filter((el) => ![...el.children].some((c) => c.getBoundingClientRect().right > edge + 0.5))
      .slice(0, 5)
      .map((el) => `<${el.tagName.toLowerCase()} class="${el.getAttribute('class') ?? ''}"> ${el.textContent.slice(0, 40)}`)
    return {
      doc: [root.scrollWidth, root.clientWidth],
      main: main ? [main.scrollWidth, main.clientWidth] : [0, 0],
      offenders,
    }
  })
}

async function expectFits(page: Page, label: string): Promise<void> {
  await page.evaluate(() => document.fonts.ready)
  const { doc, main, offenders } = await widths(page)
  expect(doc[0], `${label}: the page is ${String(doc[0] - doc[1])} px wider than the viewport`).toBeLessThanOrEqual(doc[1])
  expect(main[0], `${label}: the section scrolls ${String(main[0] - main[1])} px sideways: ${offenders.join('; ')}`).toBeLessThanOrEqual(main[1])
}

/** The open dialog neither scrolls sideways nor reaches past the viewport. */
async function expectDialogFits(page: Page, label: string): Promise<void> {
  const [scroll, client, right, viewport] = await page.getByRole('dialog').evaluate((el) => [el.scrollWidth, el.clientWidth, el.getBoundingClientRect().right, window.innerWidth])
  expect(scroll, `${label}: the dialog scrolls ${String(scroll - client)} px sideways`).toBeLessThanOrEqual(client)
  expect(right, `${label}: the dialog reaches past the viewport`).toBeLessThanOrEqual(viewport)
}

const setTheme = (page: Page, theme: 'dark' | 'light') =>
  page.evaluate((dark) => document.documentElement.classList.toggle('dark', dark), theme === 'dark')

async function shot(page: Page, name: string, theme: string, width: number): Promise<void> {
  if (process.env.KEEL_SHOTS) await page.screenshot({ path: `${process.env.KEEL_SHOTS}/${name}-${theme}-${String(width)}.png`, fullPage: true })
}

const DECRYPTS = [
  { host: 'host-keel', inventory_name: 'keel', age_recipient: AGE.keel, decrypts: ['secrets/k3s_servers.sops.yaml', 'secrets/keel_servers.sops.yaml'] },
  { host: 'host-kube1', inventory_name: 'kube1', age_recipient: AGE.kube, decrypts: [] },
  { host: 'host-steam', inventory_name: 'steam', age_recipient: null, decrypts: [] },
]

const TABS: { tab: string; item: string }[] = [
  { tab: 'playbooks', item: 'playbooks/keel-server.yaml' },
  { tab: 'roles', item: 'keel_server' },
  { tab: 'inventory', item: 'inventory/hosts.yml%23compose_hosts' },
  { tab: 'secrets', item: 'secrets/k3s_servers.sops.yaml' },
  { tab: 'jobs', item: 'k3s-master' },
]

const WIDTHS = [360, 1280] as const
const THEMES = ['dark', 'light'] as const

for (const theme of THEMES) {
  for (const width of WIDTHS) {
    test.describe(`${theme} at ${String(width)} px`, () => {
      test.use({ viewport: { width, height: 800 } })

      for (const { tab, item } of TABS) {
        test(`repository ${tab} fits the viewport`, async ({ page, backend }) => {
          setUp(backend, { decrypts: DECRYPTS })
          await page.goto(`/repository?tab=${tab}&item=${item}`)
          await setTheme(page, theme)
          await expect(page.getByTestId(`repo-tab-${tab}`).getByTestId('catalog-item').first()).toBeVisible()
          await expect(page.locator('[data-detail-pane]')).toBeVisible()
          await shot(page, `repository-${tab}`, theme, width)
          await expectFits(page, `/repository?tab=${tab}`)
        })
      }

      test('the Add host dialog fits the viewport, form and result', async ({ page, backend }) => {
        setUp(backend)
        await page.goto('/repository?tab=inventory')
        await setTheme(page, theme)
        await page.getByRole('button', { name: 'Add host' }).click()
        const dialog = page.getByRole('dialog', { name: 'Add host' })
        await dialog.getByLabel('Inventory name').fill('web-frontend-eu-west-1a.internal.example.com')
        await dialog.getByRole('button', { name: 'compose_hosts' }).click()
        await expectDialogFits(page, 'Add host form')
        await shot(page, 'add-host-form', theme, width)

        await dialog.getByRole('button', { name: 'Review' }).click()
        await confirmChange(page, 'Add host web-frontend-eu-west-1a.internal.example.com')
        await expect(page.getByRole('dialog', { name: 'Host added' }).getByTestId('change-outcome')).toBeVisible()
        await expectDialogFits(page, 'Add host result')
        await shot(page, 'add-host-result', theme, width)
        await expectFits(page, 'Add host')
      })
    })
  }
}
