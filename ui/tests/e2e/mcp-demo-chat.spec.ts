/**
 * 015-mcp-agent-exposure, US4/T052: the "Launch example chat" affordance in Available Agents.
 *
 * Three honesty rules, each with its own test, because each one is a way the panel could lie:
 *
 * 1. **Server off** — no launch button at all, only the reason. An example chat with the MCP
 *    server off would have nothing to attach to, so offering the button would promise a session
 *    the backend is guaranteed to refuse (FR-009).
 * 2. **No `claude` on PATH** — the backend says so with `McpDemoChatNotDetected` and the panel
 *    shows those instructions verbatim instead of a button. Nothing is simulated.
 * 3. **Detected** — the button sends exactly one `StartMcpDemoChat`, the real subprocess's output
 *    is what appears in the terminal, and when the process exits the panel repeats the backend's
 *    reason word for word (including a usage error from the real CLI) rather than hiding it.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { clearSent, emit, navigateTo, open, sent, sentTypes } from './helpers'

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

function encodeChunk(text: string): string {
  return Buffer.from(text, 'utf-8').toString('base64')
}

function section(page: Page) {
  return page.getByTestId('mcp-available-agents')
}

function demoChat(page: Page) {
  return page.getByTestId('mcp-demo-chat')
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
  // 043 US3: app-wide settings moved from Cluster settings to App settings.
  await navigateTo(page, 'App settings', 'Settings')
  await expect(section(page)).toBeVisible()
}

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

test.describe('US4 MCP demo chat', () => {
  test('with the server off there is nothing to launch', async ({ page }) => {
    await goToSettings(page)
    await mcpStatus(page, 2, false, false)

    await expect(demoChat(page).getByTestId('mcp-demo-chat-server-off')).toContainText(
      'Turn the MCP server on first',
    )
    await expect(demoChat(page).getByTestId('mcp-demo-chat-launch')).toHaveCount(0)
    await expect(demoChat(page).getByTestId('mcp-demo-chat-terminal')).toHaveCount(0)

    // Not even an enabled-but-not-running server gets a button: "running" is what a chat needs.
    await mcpStatus(page, 3, true, false)
    await expect(demoChat(page).getByTestId('mcp-demo-chat-server-off')).toBeVisible()
    await expect(demoChat(page).getByTestId('mcp-demo-chat-launch')).toHaveCount(0)
    expect(await sentTypes(page)).not.toContain('StartMcpDemoChat')
  })

  test('without a claude CLI on PATH the backend instructions replace the button', async ({
    page,
  }) => {
    await goToSettings(page)
    await mcpStatus(page, 2, true, true)
    await emit(
      page,
      batch([
        system(3, {
          type: 'McpDemoChatNotDetected',
          data: {
            instructions:
              'No `claude` executable was found on PATH. Install Claude Code, then reopen this panel.',
          },
        }),
      ]),
    )

    await expect(demoChat(page).getByTestId('mcp-demo-chat-not-detected')).toContainText(
      'No `claude` executable was found on PATH. Install Claude Code, then reopen this panel.',
    )
    await expect(demoChat(page).getByTestId('mcp-demo-chat-launch')).toHaveCount(0)
    expect(await sentTypes(page)).not.toContain('StartMcpDemoChat')
  })

  test('a detected CLI launches, streams the real output, and reports how it ended', async ({
    page,
  }) => {
    await goToSettings(page)
    await mcpStatus(page, 2, true, true)

    // Detected means: the backend said nothing about detection failing. The button appears.
    const launch = demoChat(page).getByTestId('mcp-demo-chat-launch')
    await expect(launch).toContainText('Launch example chat')
    await expect(demoChat(page).getByTestId('mcp-demo-chat-terminal')).toHaveCount(0)

    await clearSent(page)
    await launch.click()
    await expect.poll(() => sentTypes(page)).toContain('StartMcpDemoChat')
    expect((await sent(page)).filter((c) => c.type === 'StartMcpDemoChat')).toEqual([
      { type: 'StartMcpDemoChat' },
    ])

    await emit(page, batch([system(3, { type: 'McpDemoChatReady', data: { session_id: 1 } })]))
    const xterm = demoChat(page).getByTestId('mcp-demo-chat-xterm')
    await expect(xterm).toBeVisible()
    await expect(demoChat(page).getByTestId('mcp-demo-chat-stop')).toBeVisible()

    // The subprocess's own stdout, byte for byte, is what the terminal shows.
    await emit(
      page,
      batch([
        system(4, {
          type: 'McpDemoChatOutputChunk',
          data: { session_id: 1, data: encodeChunk('connected to fathom mcp\r\n') },
        }),
      ]),
    )
    await expect(xterm).toContainText('connected to fathom mcp')

    // Keystrokes reach the process rather than being swallowed by the panel.
    await xterm.click()
    await page.keyboard.type('what pods are unhealthy?')
    await expect.poll(() => sentTypes(page)).toContain('SendMcpDemoChatInput')

    // A previous session's trailing chunk cannot bleed into a later one.
    await emit(
      page,
      batch([
        system(5, {
          type: 'McpDemoChatOutputChunk',
          data: { session_id: 99, data: encodeChunk('FROM A DIFFERENT SESSION') },
        }),
      ]),
    )
    await expect(xterm).not.toContainText('FROM A DIFFERENT SESSION')

    // However it ended, the backend's reason is repeated verbatim — this is the honest surface
    // for a real `claude` usage error, not a swallowed failure.
    await emit(
      page,
      batch([
        system(6, {
          type: 'McpDemoChatEnded',
          data: { session_id: 1, reason: 'error: unknown option --mcp-config' },
        }),
      ]),
    )
    await expect(demoChat(page).getByTestId('mcp-demo-chat-ended')).toContainText(
      'error: unknown option --mcp-config',
    )
    await expect(demoChat(page).getByTestId('mcp-demo-chat-stop')).toHaveCount(0)
    await expect(demoChat(page).getByTestId('mcp-demo-chat-launch')).toContainText(
      'Relaunch example chat',
    )

    // Stopping is the operator's own command, and only theirs.
    await clearSent(page)
    await emit(page, batch([system(7, { type: 'McpDemoChatReady', data: { session_id: 2 } })]))
    await demoChat(page).getByTestId('mcp-demo-chat-stop').click()
    await expect.poll(() => sentTypes(page)).toContain('StopMcpDemoChat')
  })
})
