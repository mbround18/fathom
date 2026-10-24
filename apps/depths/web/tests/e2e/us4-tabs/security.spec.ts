/**
 * 051: container containment in the UI, against the fake backend. Root and escape chips on the
 * stack list, container rows and the console, with their reasons on focus and tap; shell buttons
 * disabled with the reason; the rootful-daemon notice for admins only; and, under the enforcing
 * compose policy, an escape deploy that confirms only once an admin acknowledges it.
 */
import type { Page } from '@playwright/test'

import type { ContainerSecurity, Stack } from '../../../src/protocol/generated'
import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

const ROOT: ContainerSecurity = {
  level: 'root',
  reasons: [{ code: 'root_user', severity: 'root', message: 'Runs as root (uid 0) on a rootful daemon.' }],
  shell: 'admin_only',
}
const ESCAPE: ContainerSecurity = {
  level: 'escape',
  reasons: [{ code: 'privileged', severity: 'escape', message: 'Runs privileged: it can reach the host.' }],
  shell: 'refused',
}

function rated(name: string, security: ContainerSecurity): Stack {
  const s = f.stack(name)
  for (const v of s.services) for (const c of v.containers) c.security = security
  s.security = security.level
  return s
}

function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList([rated('web', ROOT), rated('db', ESCAPE), f.stack('ok')])
}

const row = (page: Page, name: string) => page.locator(`[data-container="${name}"]`)

test.describe('051 container security', () => {
  test.beforeEach(({ backend }) => seed(backend))

  test('the list chips stacks by their worst container, and a chip says why on focus and tap', async ({ page, backend }) => {
    backend.me = sessions.user(['web', 'db', 'ok'])
    await page.goto('/stacks/web')
    const list = (n: string) => page.locator(`[data-stack="${n}"] [data-field="security"]`)
    await expect(list('web')).toHaveAttribute('data-level', 'root')
    await expect(list('db')).toHaveAttribute('data-level', 'escape')
    await expect(list('ok')).toHaveCount(0)

    const chip = row(page, 'web-app-1').locator('[data-field="security"]')
    await expect(chip).toHaveText('root')
    await chip.focus()
    await expect(page.getByRole('tooltip').filter({ hasText: 'Runs as root (uid 0)' })).toBeVisible()
    await chip.blur()
    await chip.click()
    await expect(chip).toHaveAttribute('aria-expanded', 'true')
    await expect(page.locator('[data-console] [data-field="security"]')).toHaveAttribute('data-level', 'root')
  })

  test('a stack row opens on a click anywhere but its chips, and the chip names the container and why', async ({ page, backend }) => {
    backend.me = sessions.user(['web', 'db', 'ok'])
    await page.goto('/stacks/web')
    const db = page.locator('[data-stack="db"]')
    const chip = db.locator('[data-field="security"]')
    await chip.click()
    await expect(page.getByRole('tooltip').filter({ hasText: 'db-app-1 Runs privileged: it can reach the host.' })).toBeVisible()
    await expect(page).toHaveURL(/\/stacks\/web$/)
    // The open tooltip hangs over the row; let it close as it would when the pointer moves on.
    await chip.blur()
    await page.mouse.move(0, 0)
    await expect(page.getByRole('tooltip').filter({ hasText: 'db-app-1' })).toBeHidden()
    // The name link's overlay covers the row, so the click lands on it wherever it falls.
    await db.locator('[data-field="services"]').click({ force: true })
    await expect(page).toHaveURL(/\/stacks\/db$/)
  })

  test('a user cannot open a shell in a root container; an admin can, and nobody in an escape', async ({ page, backend }) => {
    backend.me = sessions.user(['web', 'db'])
    await page.goto('/stacks/web')
    const shell = row(page, 'web-app-1').locator('[data-open="shell"]')
    await expect(shell).toHaveAttribute('aria-disabled', 'true')
    await expect(shell).toHaveAttribute('data-disabled-reason', /administrator/)
    await shell.click({ force: true })
    await expect(page.locator('[data-tab-kind="shell"]')).toHaveCount(0)
    await expect(page.locator('[data-action="console-pop-shell"]')).toHaveAttribute('aria-disabled', 'true')

    await page.goto('/stacks/db')
    await expect(row(page, 'db-app-1').locator('[data-open="shell"]')).toHaveAttribute('data-disabled-reason', /reach the host/)

    backend.me = sessions.admin()
    await page.goto('/stacks/web')
    const adminShell = row(page, 'web-app-1').locator('[data-open="shell"]')
    await expect(adminShell).not.toHaveAttribute('aria-disabled', 'true')
    await adminShell.click()
    await backend.waitForCmd('OpenShell')
    await expect(page.locator('[data-tab-kind="shell"]')).toHaveCount(1)
    await page.goto('/stacks/db')
    await expect(row(page, 'db-app-1').locator('[data-open="shell"]')).toHaveAttribute('aria-disabled', 'true')
  })

  test('admins are told the daemon runs as root; users never learn its mode', async ({ page, backend }) => {
    backend.me = sessions.admin({ features: { shell: true, edit: true, host_metrics: false, daemon: 'rootful' } })
    await page.goto('/')
    await expect(page.locator('[data-notice="rootful"]')).toContainText('runs as root')
    await expect(page.locator('[data-notice="rootful"] a')).toHaveAttribute('href', /docs\/guides\/depths\/security\.md$/)

    backend.me = sessions.admin({ features: { shell: true, edit: true, host_metrics: false, daemon: 'rootless' } })
    await page.goto('/')
    await expect(page.locator('[data-page="overview"]')).toBeVisible()
    await expect(page.locator('[data-notice="rootful"]')).toHaveCount(0)
  })

  test('under enforce, an admin deploys escape findings only by acknowledging them; a user is refused', async ({ page, backend }) => {
    backend.composePolicy = 'enforce'
    backend.lint.db = [
      { service: 'app', code: 'privileged', severity: 'escape', message: 'runs privileged: it can reach the host', line: 4 },
      { service: 'app', code: 'root_user', severity: 'root', message: 'runs as root', line: null },
    ]
    backend.me = sessions.admin()
    await page.goto('/stacks/db')
    await page.locator('[role="toolbar"][aria-label="Stack actions"] [data-action="up"]').click()
    const dialog = page.getByRole('dialog')
    await expect(dialog.locator('[data-findings="escape"]')).toBeVisible()
    await expect(dialog.locator('[data-finding="privileged"]')).toContainText('line 4')
    const confirm = dialog.locator('[data-confirm]')
    await expect(confirm).toBeDisabled()
    await dialog.locator('[data-ack] input[type="checkbox"]').check()
    await expect(confirm).toBeEnabled()
    await confirm.click()
    const sent = await backend.waitForCmd('Confirm')
    expect(sent.data.acknowledge_findings).toBe(true)
    await expect(dialog).toHaveCount(0)

    backend.me = sessions.user(['db'])
    await page.goto('/stacks/db')
    await page.locator('[role="toolbar"][aria-label="Stack actions"] [data-action="up"]').click()
    await expect(page.getByRole('dialog').locator('[data-refusal="denied"]')).toContainText('privileged')
  })

  test('the editor lints the file as it is written, and warn never blocks', async ({ page, backend }) => {
    backend.me = sessions.admin()
    const web = backend.stacks.stacks[0]
    if (!web) throw new Error('seeded')
    backend.details.web = f.detail(web, { compose_text: 'services:\n  app:\n    image: nginx:1.29\n    privileged: true\n' })
    await page.goto('/stacks/web/edit')
    const lint = await backend.waitForCmd('LintCompose')
    expect(lint.data.stack).toBe('web')
    const panel = page.locator('[data-lint-panel]')
    await expect(panel.locator('[data-finding="privileged"]')).toContainText('line 4')
    await expect(panel.locator('[data-field="security"]')).toHaveAttribute('data-level', 'escape')
  })
})
