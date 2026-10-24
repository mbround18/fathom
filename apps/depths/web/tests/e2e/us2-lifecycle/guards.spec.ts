/**
 * US2: the guards around an operation (T053): one at a time per stack, destructive options off by
 * default and named when on, and a preview that no longer matches the stack (changed, expired) is
 * refused by the server rather than run.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

const dialog = (page: Page) => page.getByRole('dialog')
const argv0 = (page: Page) => dialog(page).locator('[data-step="0"] [data-argv]')
const compose = ['docker', 'compose', '-p', 'web', '-f', 'compose.yaml']

function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList([f.stack('web')])
}

async function open(page: Page): Promise<void> {
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
}

async function preview(page: Page, action: string): Promise<void> {
  await page.locator(`[aria-label="Stack actions"] [data-action="${action}"]`).click()
  await expect(dialog(page).locator('[data-step]').first()).toBeVisible()
}

test('a second operation on a busy stack is refused, naming who runs what', async ({ page, backend }) => {
  seed(backend)
  backend.op = { output: [], outcome: { kind: 'succeeded' }, hold: true }
  await open(page)
  await preview(page, 'restart')
  await dialog(page).locator('[data-confirm]').click()
  await expect(page.locator('[data-operation="restart"]')).toHaveAttribute('data-op-state', 'running')

  await page.locator('[aria-label="Stack actions"] [data-action="stop"]').click()
  const refusal = dialog(page).locator('[data-refusal="busy"]')
  await expect(refusal).toContainText('ada (github)')
  await expect(refusal).toContainText('restart')
  // The operation's id (the console's stream took a number from the same counter).
  await expect(refusal).toContainText(/operation op-\d+/)
  await expect(dialog(page).locator('[data-confirm]')).toHaveCount(0)
  expect(backend.received.filter((c) => c.type === 'Confirm')).toHaveLength(1)

  // Once it is done, asking again gets a preview.
  backend.finishOp()
  await expect(page.locator('[data-operation="restart"]')).toHaveAttribute('data-op-state', 'succeeded')
  await dialog(page).getByRole('button', { name: 'Preview again' }).click()
  await expect(argv0(page)).toHaveAttribute('data-argv', JSON.stringify([...compose, 'stop']))
})

test('down keeps volumes by default and needs the stack name typed when an option is on', async ({ page, backend }) => {
  seed(backend)
  await open(page)
  await preview(page, 'down')
  await expect(argv0(page)).toHaveAttribute('data-argv', JSON.stringify([...compose, 'down']))
  await expect(dialog(page).locator('[data-kept]')).toHaveText('Volumes are kept.')
  const volumes = dialog(page).getByRole('switch', { name: /remove named volumes/i })
  const images = dialog(page).getByRole('switch', { name: /remove images/i })
  await expect(volumes).not.toBeChecked()
  await expect(images).not.toBeChecked()
  await expect(dialog(page).locator('[data-confirm]')).toBeEnabled()

  await volumes.click()
  const again = await backend.waitForCmd('PreviewOp')
  const next = await backend.waitForCmd('PreviewOp')
  expect(again.data.op).toEqual({ kind: 'down', options: { remove_volumes: false, remove_images: false } })
  expect(next.data.op).toEqual({ kind: 'down', options: { remove_volumes: true, remove_images: false } })
  await expect(argv0(page)).toHaveAttribute('data-argv', JSON.stringify([...compose, 'down', '--volumes']))
  await expect(dialog(page).locator('[data-destructive]')).toContainText(/volumes/i)
  await expect(dialog(page).locator('[data-kept]')).toHaveCount(0)

  // Typed confirmation: the wrong name keeps Confirm disabled.
  const confirm = dialog(page).locator('[data-confirm]')
  await expect(confirm).toBeDisabled()
  const typed = dialog(page).getByLabel(/type web to confirm/i)
  await typed.fill('wbe')
  await expect(confirm).toBeDisabled()
  await typed.fill('web')
  await expect(confirm).toBeEnabled()
  await confirm.click()
  await backend.waitForCmd('Confirm')
  await expect(page.locator('[data-operation="down"]')).toBeVisible()
})

test('a user who is not an admin cannot ask for volumes to be removed', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.user(['web'])
  await open(page)
  await preview(page, 'down')
  await expect(dialog(page).getByRole('switch', { name: /remove named volumes/i })).toHaveCount(0)
  await expect(argv0(page)).toHaveAttribute('data-argv', JSON.stringify([...compose, 'down']))
})

test('a stack changed since the preview is not run: the server voids it', async ({ page, backend }) => {
  seed(backend)
  await open(page)
  await preview(page, 'up')
  // Someone edits the compose file on disk.
  backend.stacks = f.stackList([f.stack('web', { content_hash: 'hash-web-2' })])

  await dialog(page).locator('[data-confirm]').click()
  await backend.waitForCmd('Confirm')
  await expect(dialog(page).locator('[data-refusal="void"]')).toContainText('The stack changed since this preview was made')
  await expect(page.locator('[data-operation]')).toHaveCount(0)

  // A fresh preview carries the new hash and runs.
  await dialog(page).getByRole('button', { name: 'Preview again' }).click()
  await expect(dialog(page).locator('[data-refusal]')).toHaveCount(0)
  await dialog(page).locator('[data-confirm]').click()
  await expect(page.locator('[data-operation="up"]')).toBeVisible()
})

test('an expired preview is refused and asks for a new one', async ({ page, backend }) => {
  seed(backend)
  await open(page)
  await preview(page, 'restart')
  backend.expirePreviews()
  await dialog(page).locator('[data-confirm]').click()
  await expect(dialog(page).locator('[data-refusal="void"]')).toContainText('expired')
  await expect(dialog(page).locator('[data-confirm]')).toHaveCount(0)
  await expect(page.locator('[data-operation]')).toHaveCount(0)
})

test('an operation the server denies says so and runs nothing', async ({ page, backend }) => {
  seed(backend)
  backend.deny('PreviewOp')
  await open(page)
  await page.locator('[aria-label="Stack actions"] [data-action="up"]').click()
  await expect(dialog(page).locator('[data-refusal="denied"]')).toHaveText('You are not permitted to do this.')
  await expect(dialog(page).locator('[data-confirm]')).toHaveCount(0)
})
