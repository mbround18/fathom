/**
 * 015-mcp-agent-exposure, US1: the "Available Agents" section of Cluster Settings.
 *
 * Walks the operator's whole journey in one pass: the server starts **off** (nothing listening,
 * no connection snippet), the toggle is the one thing that asks the backend to enable it, and
 * once the backend reports `running` the connection command an external MCP client should launch
 * appears.
 *
 * The off-state assertions are the point: mounting the panel must send only `LoadMcpServerStatus`
 * (a read), never `SetMcpServerEnabled` (FR-002).
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, finish, navigateTo, nextRequestId, open, sentTypes } from './helpers'

const ACTIVE_CONTEXT = 'kind-a'
const CONNECTION_COMMAND = '/usr/local/bin/fathom --only-mcp'

const BASE_SETTINGS = {
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

function section(page: Page) {
  return page.getByTestId('mcp-available-agents')
}

async function goToSettings(page: Page) {
  await open(page)
  await navigateTo(page, 'Cluster Settings', 'Team')
  await expect.poll(() => sentTypes(page)).toContain('LoadClusterSettings')
  await emit(
    page,
    batch([
      system(1, {
        type: 'ClusterSettingsLoaded',
        data: { context: ACTIVE_CONTEXT, settings: BASE_SETTINGS },
      }),
    ]),
  )
  await expect(section(page)).toBeVisible()
}

/** The backend's answer to `LoadMcpServerStatus` / broadcast after `SetMcpServerEnabled`. */
async function mcpStatus(page: Page, seq: number, enabled: boolean, running: boolean) {
  await emit(
    page,
    batch([
      system(seq, {
        type: 'McpServerStatusChanged',
        data: { enabled, running, connection_command: CONNECTION_COMMAND },
      }),
    ]),
  )
}

test.describe('US1 MCP agent exposure: Available Agents status', () => {
  test('starts off, toggles on, and reports running with a connection command', async ({ page }) => {
    await goToSettings(page)

    // Mounting the section reads status and never enables anything.
    await expect.poll(() => sentTypes(page)).toContain('LoadMcpServerStatus')
    expect(await sentTypes(page)).not.toContain('SetMcpServerEnabled')

    await mcpStatus(page, 2, false, false)
    await expect(section(page).getByTestId('mcp-server-toggle')).not.toBeChecked()
    await expect(section(page).getByTestId('mcp-server-state')).toContainText('Off')
    await expect(section(page).getByTestId('mcp-connection-command')).toHaveCount(0)
    await expect(section(page).getByTestId('mcp-running-indicator')).toHaveCount(0)

    // The operator turns it on: exactly one explicit enable command.
    const requestId = await nextRequestId(page)
    // `.click()`, not `.check()`: the toggle is controlled by the store, so it only flips once
    // the backend broadcasts the new status — Playwright's `check()` would fail waiting for it.
    await section(page).getByTestId('mcp-server-toggle').click()
    await expect.poll(() => sentTypes(page)).toContain('SetMcpServerEnabled')
    const cmds = await page.evaluate(() => window.__fathomFake?.sent() ?? [])
    const toggles = cmds.filter((c) => c.type === 'SetMcpServerEnabled')
    expect(toggles).toEqual([{ type: 'SetMcpServerEnabled', data: { enabled: true } }])
    await finish(page, requestId, true)

    // The backend then broadcasts the new status, and the snippet appears.
    await mcpStatus(page, 3, true, true)
    await expect(section(page).getByTestId('mcp-server-toggle')).toBeChecked()
    await expect(section(page).getByTestId('mcp-running-indicator')).toContainText('Running')
    await expect(section(page).getByTestId('mcp-connection-command')).toContainText(
      CONNECTION_COMMAND,
    )
  })

  test('an enabled server that is not running is not reported as healthy', async ({ page }) => {
    await goToSettings(page)
    await mcpStatus(page, 2, true, false)

    await expect(section(page).getByTestId('mcp-server-state')).toContainText(
      'Enabled, but not running.',
    )
    await expect(section(page).getByTestId('mcp-running-indicator')).toHaveCount(0)
  })

  test('turning it back off asks the backend to stop and clears the snippet', async ({ page }) => {
    await goToSettings(page)
    await mcpStatus(page, 2, true, true)
    await expect(section(page).getByTestId('mcp-connection-command')).toBeVisible()

    const requestId = await nextRequestId(page)
    await section(page).getByTestId('mcp-server-toggle').click()
    const cmds = await page.evaluate(() => window.__fathomFake?.sent() ?? [])
    expect(cmds).toContainEqual({ type: 'SetMcpServerEnabled', data: { enabled: false } })
    await finish(page, requestId, true)

    await mcpStatus(page, 3, false, false)
    await expect(section(page).getByTestId('mcp-connection-command')).toHaveCount(0)
    await expect(section(page).getByTestId('mcp-server-state')).toContainText('Off')
  })
})
