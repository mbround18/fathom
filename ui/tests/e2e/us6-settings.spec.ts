/**
 * US6 (T113): a saved (egui-era shaped) config renders in Cluster Settings, the MCP toggle sends
 * `SetMcpServerEnabled`, saved theme prefs reach `<html>` and reduced motion stops animations.
 * There is no in-app theme picker in the current UI (`store.setTheme` has no caller), so the
 * theme prefs arrive via `UiConfigLoaded`, the same path a relaunch takes. The MCP restart
 * round trip itself is covered by mcp-settings-persistence.spec.ts.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import type { UiConfig } from '../../src/protocol/types'
import { emit, navigateTo, nextRequestId, finish, open, sentTypes } from './helpers'

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
    const running = await page.evaluate(
      () =>
        document
          .getAnimations()
          .filter(
            (a) =>
              a.playState === 'running' && (a.effect?.getTiming().iterations ?? 1) === Infinity,
          )
          .map((a) => (a as CSSAnimation).animationName),
    )
    expect(running).toEqual([])
  })
})
