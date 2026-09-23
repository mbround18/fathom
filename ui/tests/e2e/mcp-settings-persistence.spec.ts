/**
 * 015-mcp-agent-exposure, US3/T042: the MCP server preference survives a restart.
 *
 * The acceptance scenario is about what the operator *sees*, not about the file on disk (the
 * `~/.fathom/config.json` half is covered by `tests/ui_config_persistence.rs`): after turning the
 * server on and restarting Fathom, the Available Agents toggle must already render **on**, from
 * the preference the backend replays in `UiConfigLoaded`, without the operator touching it again
 * and without the UI sending a single `SetMcpServerEnabled`.
 *
 * The restart is a full `page.goto` — a fresh store, a fresh transport and a fresh mount, which
 * is exactly what a relaunched app is from the UI's side. What stands in for the persisted file
 * is the backend re-reporting `mcp_stdio_enabled: true`.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { clearSent, emit, finish, navigateTo, nextRequestId, open, sentTypes } from './helpers'

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

/**
 * The preference a restarted backend replays while loading the UI config, before it has had a
 * chance to broadcast any live server status.
 */
async function replayPersistedPreference(page: Page, enabled: boolean) {
  await expect.poll(() => sentTypes(page)).toContain('LoadUiConfig')
  await emit(
    page,
    batch([
      system(50, {
        type: 'UiConfigLoaded',
        data: { ui: null, expanded_groups: ['Workloads', 'Team'], mcp_stdio_enabled: enabled },
      }),
    ]),
  )
}

test.describe('US3 MCP settings persistence', () => {
  test('a server turned on stays on across a restart, with no operator re-input', async ({
    page,
  }) => {
    // --- First run: the operator turns the server on.
    await goToSettings(page)
    await emit(
      page,
      batch([
        system(2, {
          type: 'McpServerStatusChanged',
          data: { enabled: false, running: false, connection_command: CONNECTION_COMMAND },
        }),
      ]),
    )
    await expect(section(page).getByTestId('mcp-server-toggle')).not.toBeChecked()

    const requestId = await nextRequestId(page)
    await section(page).getByTestId('mcp-server-toggle').click()
    await expect.poll(() => sentTypes(page)).toContain('SetMcpServerEnabled')
    await finish(page, requestId, true)
    await emit(
      page,
      batch([
        system(3, {
          type: 'McpServerStatusChanged',
          data: { enabled: true, running: true, connection_command: CONNECTION_COMMAND },
        }),
      ]),
    )
    await expect(section(page).getByTestId('mcp-server-toggle')).toBeChecked()

    // --- Restart: a brand new page, store and transport, with the backend replaying the saved
    // preference exactly as it would after reading `~/.fathom/config.json`.
    await goToSettings(page)
    await replayPersistedPreference(page, true)
    await clearSent(page)

    // FR-014 / US3 acceptance scenario 2: on, before any live status has arrived and without
    // anybody flipping anything.
    await expect(section(page).getByTestId('mcp-server-toggle')).toBeChecked()
    await expect(section(page).getByTestId('mcp-server-state')).toContainText('Enabled')
    expect(await sentTypes(page)).not.toContain('SetMcpServerEnabled')

    // The live status still wins once it lands — the persisted value is only the opening guess,
    // so a server that failed to start is reported honestly instead of as "on".
    await emit(
      page,
      batch([
        system(51, {
          type: 'McpServerStatusChanged',
          data: { enabled: false, running: false, connection_command: '' },
        }),
      ]),
    )
    await expect(section(page).getByTestId('mcp-server-toggle')).not.toBeChecked()
    await expect(section(page).getByTestId('mcp-server-state')).toContainText('Off')
    expect(await sentTypes(page)).not.toContain('SetMcpServerEnabled')
  })

  test('a server left off stays off across a restart', async ({ page }) => {
    await goToSettings(page)
    await replayPersistedPreference(page, false)
    await clearSent(page)

    await expect(section(page).getByTestId('mcp-server-toggle')).not.toBeChecked()
    await expect(section(page).getByTestId('mcp-server-state')).toContainText(
      'nothing is listening',
    )
    expect(await sentTypes(page)).not.toContain('SetMcpServerEnabled')
  })
})
