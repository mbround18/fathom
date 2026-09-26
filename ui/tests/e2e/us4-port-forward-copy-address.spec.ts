/**
 * T023 (US4 Phase 6): copying an active forward's local address. Covers spec.md US4 Acceptance
 * Scenarios 1-2: selecting the copy action on an `Active` row copies exactly
 * `localhost:<local_port>` to the clipboard and shows a brief confirmation; a row that is not yet
 * `Active` (e.g. `Connecting`) disables the copy action rather than silently copying a
 * non-functional address (chosen implementation per `PortForwardsPanel.tsx`'s `canCopy` gate).
 * All new `data-testid`s are prefixed `port-forward-` for isolation from sibling 007/008/010
 * specs.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, globalRoute, list } from '../fixtures/builders'
import type { PortForward } from '../../src/protocol/types'
import {
  emit,
  navigateTo,
  prepareClipboard,
  readClipboard,
  sentTypes,
  open as openApp,
} from './helpers'

function pf(overrides: Partial<PortForward> = {}): PortForward {
  return {
    id: 'pf-1',
    namespace: 'default',
    pod: 'web-1',
    local_port: 8080,
    remote_port: 8080,
    container: 'web',
    status: 'Active',
    auto_start: false,
    ...overrides,
  }
}

async function openPortForwardsPanel(page: Page): Promise<void> {
  await openApp(page)
  await navigateTo(page, 'Port Forwards', 'Network')
  await expect.poll(() => sentTypes(page)).toContain('RefreshPortForwards')
  await page.waitForTimeout(600)
}

test.describe('US4 port forward copy address', () => {
  test('Scenario 1: copying an Active row copies exactly localhost:<local_port> and confirms', async ({
    page,
    context,
    browserName,
  }) => {
    await prepareClipboard(page, context, browserName)
    await openPortForwardsPanel(page)
    await emit(
      page,
      batch([
        list(1, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [pf({ status: 'Active', local_port: 8080 })],
        }),
      ]),
    )
    const forwardRow = page.getByTestId('port-forward-row')
    const copyButton = forwardRow.getByTestId('port-forward-copy-address')
    await expect(copyButton).toBeEnabled()
    await copyButton.click()

    await expect(page.getByTestId('port-forward-copy-toast').first()).toBeVisible()
    // The copy handler writes asynchronously; wait for the write rather than reading once.
    await expect.poll(() => readClipboard(page, browserName)).toBe('localhost:8080')
  })

  test('Scenario 2: a Connecting row disables the copy action rather than copying silently', async ({
    page,
  }) => {
    await openPortForwardsPanel(page)
    await emit(
      page,
      batch([
        list(1, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [pf({ status: 'Connecting' })],
        }),
      ]),
    )
    const forwardRow = page.getByTestId('port-forward-row')
    await expect(forwardRow).toContainText('Connecting')
    await expect(forwardRow.getByTestId('port-forward-copy-address')).toBeDisabled()
  })
})
