/**
 * US6 (T113): a saved (egui-era shaped) config renders in Cluster Settings, the MCP toggle sends
 * `SetMcpServerEnabled`, saved theme prefs reach `<html>` and reduced motion stops animations.
 * There is no in-app theme picker in the current UI (`store.setTheme` has no caller), so the
 * theme prefs arrive via `UiConfigLoaded`, the same path a relaunch takes. The MCP restart
 * round trip itself is covered by mcp-settings-persistence.spec.ts.
 */
import { readFileSync } from 'node:fs'

import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import type { UiConfig } from '../../src/protocol/types'
import { emit, navigateTo, nextRequestId, finish, open, sent, sentTypes } from './helpers'

const SETTINGS = {
  schema: 1 as const,
  metricsSource: 'none' as const,
  prometheusPath: '/api/v1',
  https: false,
  pathPrefix: '',
  hiddenMetrics: [],
  orgRevKnown: null,
  fieldOrigin: {
    metricsSource: 'local' as const,
    prometheusPath: 'local' as const,
    https: 'local' as const,
    pathPrefix: 'local' as const,
    hiddenMetrics: 'local' as const,
  },
}

const SAVED: UiConfig = {
  schema: 1,
  theme: { mode: 'light', accent: 'magenta', effects: 'off', glitch: true },
  density: {},
  layouts: {},
}

async function openSettings(page: Page) {
  await open(page)
  await expect.poll(() => sentTypes(page)).toContain('LoadUiConfig')
  await emit(
    page,
    batch([
      system(50, {
        type: 'UiConfigLoaded',
        data: { ui: SAVED, expanded_groups: ['Workloads', 'Team'], mcp_stdio_enabled: true },
      }),
    ]),
  )
  await navigateTo(page, 'Cluster Settings', 'Team')
  await expect.poll(() => sentTypes(page)).toContain('LoadClusterSettings')
  await emit(
    page,
    batch([
      system(51, {
        type: 'ClusterSettingsLoaded',
        data: { context: 'kind-a', settings: SETTINGS },
      }),
    ]),
  )
  await expect(page.getByTestId('mcp-available-agents')).toBeVisible()
}

test.describe('US6 settings', () => {
  test('saved MCP auto-start renders on and toggling it sends SetMcpServerEnabled', async ({
    page,
  }) => {
    await openSettings(page)
    const toggle = page.getByTestId('mcp-available-agents').getByTestId('mcp-server-toggle')
    await expect(toggle).toBeChecked()

    const id = await nextRequestId(page)
    await toggle.click()
    await expect.poll(() => sentTypes(page)).toContain('SetMcpServerEnabled')
    await finish(page, id, true)
  })

  test('saved theme prefs are applied to <html>', async ({ page }) => {
    await openSettings(page)
    const html = page.locator('html')
    await expect(html).toHaveAttribute('data-accent', 'magenta')
    await expect(html).toHaveAttribute('data-effects', 'off')
    await expect(html).toHaveAttribute('data-glitch', 'on')
    await expect(html).not.toHaveClass(/\bdark\b/)
  })

  test('reduced motion leaves no running animation, even with glitch on', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' })
    await openSettings(page)
    await expect(page.locator('html')).toHaveAttribute('data-glitch', 'on')
    // Let any entry transitions finish before sampling.
    await page.waitForTimeout(500)
    const running = await page.evaluate(() =>
      document
        .getAnimations()
        .filter(
          (a) => a.playState === 'running' && (a.effect?.getTiming().iterations ?? 1) === Infinity,
        )
        .map((a) => (a as CSSAnimation).animationName),
    )
    expect(running).toEqual([])
  })

  test('appearance controls change <html> immediately and persist via SaveUiConfig', async ({
    page,
  }) => {
    await openSettings(page)
    const panel = page.getByTestId('appearance-settings')
    const html = page.locator('html')

    await panel.getByLabel('Mode').selectOption('dark')
    await expect(html).toHaveClass(/\bdark\b/)
    await panel.getByLabel('Accent').selectOption('cyan')
    await expect(html).toHaveAttribute('data-accent', 'cyan')
    await panel.getByLabel('Effects').selectOption('on')
    await expect(html).toHaveAttribute('data-effects', 'on')
    await panel.getByLabel('Glitch effect').uncheck()
    await expect(html).toHaveAttribute('data-glitch', 'off')
    await panel.getByLabel('Mode').selectOption('light')
    await expect(html).not.toHaveClass(/\bdark\b/)
    await panel.getByLabel('Accent').selectOption('magenta')
    await panel.getByLabel('Effects').selectOption('off')
    await panel.getByLabel('Glitch effect').check()

    await expect
      .poll(async () => {
        const saves = (await sent(page, true)).filter((c) => c.type === 'SaveUiConfig')
        const last = saves.at(-1)
        return last?.type === 'SaveUiConfig' ? last.data.ui.theme : undefined
      })
      .toEqual({ mode: 'light', accent: 'magenta', effects: 'off', glitch: true })
  })

  test('density override retunes row height on pointer classes and persists', async ({ page }) => {
    await openSettings(page)
    const panel = page.getByTestId('appearance-settings')
    const rowHeight = () =>
      page.evaluate(() =>
        Number.parseInt(document.documentElement.style.getPropertyValue('--row-height'), 10),
      )
    const touch = ['phone', 'tablet'].includes(
      (await page.locator('html').getAttribute('data-width-class')) ?? '',
    )
    const auto = await rowHeight()

    await panel.getByLabel('Density').selectOption('comfortable')
    const comfortable = touch ? auto : 36
    await expect.poll(rowHeight).toBe(comfortable)
    await panel.getByLabel('Density').selectOption('compact')
    // Touch classes keep the 44px touch target (FR-012); pointer classes shrink.
    await expect.poll(rowHeight).toBe(touch ? auto : 22)
    await expect
      .poll(async () => {
        const last = (await sent(page, true)).filter((c) => c.type === 'SaveUiConfig').at(-1)
        return last?.type === 'SaveUiConfig' ? last.data.ui.density.override : undefined
      })
      .toBe('compact')
    await panel.getByLabel('Density').selectOption('auto')
    await expect.poll(rowHeight).toBe(auto)
  })

  test('shortcuts reference and About show the real app version', async ({ page }) => {
    await openSettings(page)
    await expect(page.getByTestId('shortcuts')).toContainText('Ctrl/Cmd + K')
    await expect(page.getByTestId('shortcuts')).toContainText('Esc')
    const conf = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')) as {
      version: string
    }
    await expect(page.getByTestId('about-version')).toHaveText(`v${conf.version}`)
  })
})
