/**
 * Spec 053 T074, the Files tab screenshots for docs/guides/depths/files.md, taken against the fake
 * backend with fixture stacks only, so no hostname, account or real stack reaches an image.
 *
 * Not part of the e2e suite: `DEPTHS_PROMO=1 pnpm exec playwright test --project promo`.
 */
import path from 'node:path'

import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../support/fake-backend'

const OUT = path.resolve(import.meta.dirname, '../../../../../docs/guides/depths/img')
const STACK = 'minecraft'

const fileView = (page: Page) => page.locator('[data-pane="file"]')
const listing = (page: Page) => page.locator('[data-pane="listing"]')
const editor = (page: Page) => fileView(page).locator('[data-code-editor] .cm-content')

const PROPS = [
  '# Minecraft server properties',
  'motd=A Depths server',
  'difficulty=normal',
  'gamemode=survival',
  'max-players=20',
  'view-distance=10',
  'white-list=true',
  '',
].join('\n')

function seed(backend: FakeBackend): string {
  backend.stacks = f.stackList([f.stack(STACK)])
  const data = backend.files.addMount(STACK, { source: 'minecraft_data' })
  backend.files.addMount(STACK, {
    kind: 'bind',
    source: '/srv/stacks/minecraft/plugins',
    services: [{ service: 'app', destination: '/plugins', read_only: true }],
    writable: false,
  })
  backend.files.keepVersion(STACK, data, 'server.properties', PROPS.replace('max-players=20', 'max-players=10'))
  backend.files.keepVersion(STACK, data, 'server.properties', PROPS.replace('white-list=true', 'white-list=false'))
  backend.files.write(STACK, data, 'server.properties', PROPS)
  backend.files.write(STACK, data, 'ops.json', '[\n  { "name": "admin", "level": 4 }\n]\n')
  backend.files.write(STACK, data, 'whitelist.json', '[]\n')
  backend.files.write(STACK, data, 'config/paper-global.yml', 'chunk-loading:\n  autoconfig-send-distance: true\n')
  backend.files.mkdir(STACK, data, 'world/region')
  backend.files.write(STACK, data, 'world/level.dat', new Uint8Array([0x1f, 0x8b, 0x08, 0x00]))
  backend.files.mkdir(STACK, data, 'logs')
  backend.files.write(STACK, data, 'logs/latest.log', '[12:00:00 INFO]: Done (3.2s)!\n')
  backend.files.setRunning(STACK, data, ['app'])
  return data
}

async function shot(page: Page, name: string): Promise<void> {
  await page.waitForTimeout(250)
  await page.screenshot({ path: path.join(OUT, `${name}.png`) })
}

test.use({ viewport: { width: 1440, height: 900 } })

test('files: listing, editor, save preview, history', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/${STACK}/files/${data}`)
  await expect(listing(page).locator('[data-entry="server.properties"]')).toBeVisible()
  await shot(page, 'files-listing')

  await page.goto(`/stacks/${STACK}/files/${data}/server.properties`)
  await expect(editor(page)).toContainText('motd=A Depths server')
  await shot(page, 'files-editor')

  await editor(page).click()
  await page.keyboard.press('ControlOrMeta+a')
  await page.keyboard.insertText(PROPS.replace('difficulty=normal', 'difficulty=hard').replace('max-players=20', 'max-players=30'))
  await fileView(page).locator('[data-save]').click()
  await expect(page.locator('[data-file-op] [data-diff-line="added"]').first()).toBeVisible()
  await shot(page, 'files-save-preview')
  await page.keyboard.press('Escape')
  await expect(page.locator('[data-file-op]')).toHaveCount(0)

  await page.goto(`/stacks/${STACK}/files/${data}/server.properties`)
  await expect(editor(page)).toContainText('motd=A Depths server')
  await fileView(page).locator('[data-history-toggle]').click()
  await expect(fileView(page).locator('[data-history] [data-version]')).toHaveCount(2)
  await shot(page, 'files-history')
})

test.describe('phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true })

  test('files: phone listing and editor', async ({ page, backend }) => {
    const data = seed(backend)
    await page.goto(`/stacks/${STACK}/files/${data}`)
    await expect(listing(page).locator('[data-entry="server.properties"]')).toBeVisible()
    await shot(page, 'files-phone-listing')
    await page.goto(`/stacks/${STACK}/files/${data}/server.properties`)
    await expect(editor(page)).toContainText('motd=A Depths server')
    await shot(page, 'files-phone-editor')
  })
})
