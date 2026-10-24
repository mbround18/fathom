/**
 * US2: lifecycle operations with a preview (T052), against the fake backend. The preview shows
 * the server's argv verbatim; nothing runs until Confirm; the status moves only when the server's
 * `StacksChanged` says so.
 */
import type { Page } from '@playwright/test'

import type { StackOp } from '../../../src/protocol/generated'
import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const dialog = (page: Page) => page.getByRole('dialog')
const argvOf = (page: Page, i: number) => dialog(page).locator(`[data-step="${String(i)}"] [data-argv]`)

function seed(backend: FakeBackend): void {
  const web = f.stack('web', {
    services: [
      { name: 'app', image: 'nginx:1.29', containers: [f.container({ name: 'web-app-1', project: 'web', service: 'app' })] },
      { name: 'db', image: 'postgres:18', containers: [f.container({ name: 'web-db-1', project: 'web', service: 'db' })] },
    ],
    running_services: 2,
    total_services: 2,
  })
  backend.stacks = f.stackList([web, f.stack('broken', { status: 'invalid' })])
}

async function open(page: Page, stack = 'web'): Promise<void> {
  await page.goto(`/stacks/${stack}`)
  await expect(page.getByRole('heading', { name: stack, level: 1 })).toBeVisible()
}

const compose = ['docker', 'compose', '-p', 'web', '-f', 'compose.yaml']
const CASES: { action: string; label: string; op: StackOp; argv: string[][] }[] = [
  { action: 'up', label: 'Up', op: { kind: 'up' }, argv: [[...compose, 'up', '-d', '--remove-orphans']] },
  { action: 'stop', label: 'Stop', op: { kind: 'stop' }, argv: [[...compose, 'stop']] },
  { action: 'restart', label: 'Restart', op: { kind: 'restart' }, argv: [[...compose, 'restart']] },
  {
    action: 'down',
    label: 'Down',
    op: { kind: 'down', options: { remove_volumes: false, remove_images: false } },
    argv: [[...compose, 'down']],
  },
  { action: 'pull', label: 'Pull', op: { kind: 'pull' }, argv: [[...compose, 'pull']] },
  {
    action: 'update',
    label: 'Update',
    op: { kind: 'update' },
    argv: [[...compose, 'pull'], [...compose, 'up', '-d', '--remove-orphans']],
  },
]

for (const c of CASES) {
  test(`${c.label} previews the server's exact argv, cwd and reach, and Cancel runs nothing`, async ({ page, backend }) => {
    seed(backend)
    await open(page)
    await page.locator(`[role="toolbar"][aria-label="Stack actions"] [data-action="${c.action}"]`).click()

    const cmd = await backend.waitForCmd('PreviewOp')
    expect(cmd.data).toEqual({ stack: 'web', op: c.op })

    await expect(dialog(page)).toHaveAttribute('data-preview', c.op.kind)
    await expect(dialog(page).locator('[data-step]')).toHaveCount(c.argv.length)
    for (const [i, argv] of c.argv.entries()) {
      // Element for element, as the server sent it.
      await expect(argvOf(page, i)).toHaveAttribute('data-argv', JSON.stringify(argv))
      await expect(argvOf(page, i)).toHaveText(argv.join(' '))
      await expect(dialog(page).locator(`[data-step="${String(i)}"] [data-field="cwd"]`)).toContainText('/srv/stacks/web')
    }
    await expect(dialog(page).locator('[data-reach="containers"]')).toContainText('web-app-1')
    await expect(dialog(page).locator('[data-reach="containers"]')).toContainText('web-db-1')
    await expect(dialog(page).locator('[data-reach="networks"]')).toContainText('web_default')
    await expect(dialog(page).locator('[data-destructive]')).toHaveCount(0)

    await dialog(page).getByRole('button', { name: 'Cancel' }).click()
    await expect(dialog(page)).toHaveCount(0)
    expect(backend.received.filter((x) => x.type === 'Confirm')).toHaveLength(0)
    await expect(page.locator('[data-operation]')).toHaveCount(0)
  })
}

test('per-service start, stop and restart name only that service', async ({ page, backend }) => {
  seed(backend)
  await open(page)
  const kinds = [
    ['Start db', 'service_up', ['up', '-d', 'db']],
    ['Stop db', 'service_stop', ['stop', 'db']],
    ['Restart db', 'service_restart', ['restart', 'db']],
  ] as const
  for (const [name, kind, tail] of kinds) {
    await page.getByRole('button', { name, exact: true }).click()
    const cmd = await backend.waitForCmd('PreviewOp')
    expect(cmd.data).toEqual({ stack: 'web', op: { kind, options: { service: 'db' } } })
    await expect(dialog(page)).toHaveAttribute('data-preview', kind)
    await expect(dialog(page).getByRole('heading', { level: 2 })).toHaveText(new RegExp(`db in web$`))
    await expect(argvOf(page, 0)).toHaveAttribute('data-argv', JSON.stringify([...compose, ...tail]))
    await dialog(page).getByRole('button', { name: 'Cancel' }).click()
    await expect(dialog(page)).toHaveCount(0)
  }
  expect(backend.received.filter((x) => x.type === 'Confirm')).toHaveLength(0)
})

test('an argv element with spaces or quotes is shown quoted and kept whole', async ({ page, backend }) => {
  seed(backend)
  const argv = ['docker', 'compose', '-p', 'web', 'exec', 'app', 'sh', '-c', "echo 'hi there' && $HOME"]
  backend.on('PreviewOp', (_cmd, { backend: b }) => [
    {
      type: 'Preview',
      data: {
        preview_id: 'pv-q',
        stack: 'web',
        kind: 'up',
        steps: [{ argv, cwd: '/srv/stacks/web', stdin: 'none' }],
        file_diff: null,
        reaches: { containers: [], networks: [], volumes: [] },
        destructive: false,
        stack_hash: b.stacks.stacks[0]?.content_hash ?? '',
        expires_at: '2026-10-23T12:05:00Z',
      },
    },
  ])
  await open(page)
  await page.locator('[data-action="up"]').first().click()
  await expect(argvOf(page, 0)).toHaveAttribute('data-argv', JSON.stringify(argv))
  await expect(argvOf(page, 0)).toHaveText(`docker compose -p web exec app sh -c 'echo '\\''hi there'\\'' && $HOME'`)
})

test('confirming streams output live; the status moves only on StacksChanged', async ({ page, backend }) => {
  seed(backend)
  backend.op = { output: ['Container web-app-1  Stopping\n'], outcome: { kind: 'succeeded' }, hold: true }
  await open(page)
  await page.locator('[data-action="stop"]').first().click()
  await dialog(page).locator('[data-confirm]').click()

  const confirm = await backend.waitForCmd('Confirm')
  expect(confirm.data.preview_id).toMatch(/^pv-/)
  await expect(dialog(page)).toHaveCount(0)

  const op = page.locator('[data-operation="stop"]')
  await expect(op).toHaveAttribute('data-op-state', 'running')
  await expect(op.locator('[data-op-output]')).toContainText('Container web-app-1  Stopping')
  backend.push({ type: 'OpOutput', data: { operation_id: backend.heldOps()[0] ?? '', chunk: 'Container web-app-1  Stopped\n' } })
  await expect(op.locator('[data-op-output]')).toContainText('Container web-app-1  Stopped')

  // Finished, but the status is what the server last said until it says otherwise.
  backend.finishOp({ kind: 'succeeded' })
  await expect(op).toHaveAttribute('data-op-state', 'succeeded')
  await expect(op.locator('[data-field="op-status"]')).toHaveText('Succeeded')
  await expect(page.locator('header [data-status]').first()).toHaveAttribute('data-status', 'running')

  const stopped = f.stack('web', { status: 'stopped', running_services: 0, total_services: 2 })
  backend.stacks = f.stackList([stopped])
  backend.push({ type: 'StacksChanged', data: backend.stacks })
  await expect(page.locator('header [data-status]').first()).toHaveAttribute('data-status', 'stopped')
})

test('a failed operation shows docker stderr verbatim', async ({ page, backend }) => {
  seed(backend)
  const stderr = 'Error response from daemon: driver failed programming external connectivity on endpoint web-app-1:\nBind for 0.0.0.0:8080 failed: port is already allocated'
  backend.op = { output: ['Container web-app-1  Starting\n'], outcome: { kind: 'failed', exit_code: 1, stderr }, hold: false }
  await open(page)
  await page.locator('[data-action="up"]').first().click()
  await dialog(page).locator('[data-confirm]').click()

  const op = page.locator('[data-operation="up"]')
  await expect(op).toHaveAttribute('data-op-state', 'failed')
  await expect(op.locator('[data-field="op-status"]')).toHaveText('Failed (exit 1)')
  expect(await op.locator('[data-field="stderr"]').textContent()).toBe(stderr)
})

test('a running operation can be cancelled', async ({ page, backend }) => {
  seed(backend)
  backend.op = { output: ['Pulling app\n'], outcome: { kind: 'succeeded' }, hold: true }
  await open(page)
  await page.locator('[data-action="pull"]').first().click()
  await dialog(page).locator('[data-confirm]').click()
  const op = page.locator('[data-operation="pull"]')
  await expect(op).toHaveAttribute('data-op-state', 'running')

  await op.getByRole('button', { name: 'Cancel' }).click()
  const cancel = await backend.waitForCmd('Cancel')
  expect(cancel.data.operation_id).toMatch(/^op-/)
  await expect(op).toHaveAttribute('data-op-state', 'cancelled')
  await op.getByRole('button', { name: 'Dismiss' }).click()
  await expect(op).toHaveCount(0)
})

test('a stack that does not parse offers no lifecycle actions', async ({ page, backend }) => {
  seed(backend)
  await open(page, 'broken')
  // Deleting is not a lifecycle action; an admin can still remove the stack (US3). The log
  // console (and its pop-outs) is not one either.
  await expect(page.locator('[data-page="stack"] [data-action]:not([data-action="delete"]):not([data-action^="console-"])')).toHaveCount(0)
  await expect(page.getByRole('toolbar', { name: 'Stack actions' })).toHaveCount(0)
})

test('at 360 px the preview and output fit without horizontal scroll', async ({ page, backend }) => {
  seed(backend)
  backend.op = { output: ['x'.repeat(400) + '\n'], outcome: { kind: 'succeeded' }, hold: true }
  await page.setViewportSize({ width: 360, height: 740 })
  await open(page)
  await page.locator('[data-action="update"]').first().click()
  await expect(dialog(page).locator('[data-step]')).toHaveCount(2)
  const overflow = () => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
  expect(await overflow()).toBeLessThanOrEqual(0)
  await dialog(page).locator('[data-confirm]').click()
  await expect(page.locator('[data-operation="update"]')).toBeVisible()
  expect(await overflow()).toBeLessThanOrEqual(0)
})

test('the preview traps focus and Escape cancels it', async ({ page, backend }) => {
  seed(backend)
  await open(page)
  await page.locator('[data-action="restart"]').first().click()
  await expect(dialog(page).locator('[data-confirm]')).toBeEnabled()
  for (let i = 0; i < 12; i++) {
    await page.keyboard.press('Tab')
    expect(await dialog(page).evaluate((d) => d.contains(document.activeElement))).toBe(true)
  }
  await page.keyboard.press('Escape')
  await expect(dialog(page)).toHaveCount(0)
  expect(backend.received.filter((x) => x.type === 'Confirm')).toHaveLength(0)
})
