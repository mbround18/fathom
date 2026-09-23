/**
 * 012-incluster-daemon-federation P1 (T018): the always-visible context strip and its live
 * per-context connection status (FR-001, FR-002, FR-003, FR-004, FR-005).
 */
import { expect, test, type Page } from '@playwright/test'

import {
  FEDERATION_ERROR_CONTEXT,
  FEDERATION_ERROR_MESSAGE,
  federationContexts,
  federationStatuses,
} from '../fixtures'
import { batch, system } from '../fixtures/builders'
import { emit, open, sent } from './helpers'

const switcher = (page: Page) => page.getByTestId('federation-context-switcher')

const item = (page: Page, name: string) =>
  switcher(page).locator(`[data-testid="federation-context-item"][data-context-name="${name}"]`)

test.describe('US1 federation context status strip', () => {
  test('every context appears without any open/click action (FR-001)', async ({ page }) => {
    await open(page, 'manual')
    await emit(page, federationContexts)
    await emit(page, federationStatuses)
    await expect(switcher(page)).toBeVisible()
    await expect(switcher(page).getByTestId('federation-context-item')).toHaveCount(3)
  })

  test('each item reflects its fixture status (FR-002)', async ({ page }) => {
    await open(page, 'manual')
    await emit(page, federationContexts)
    await emit(page, federationStatuses)
    await expect(item(page, 'kind-a')).toHaveAttribute('data-context-status', 'connected')
    await expect(item(page, 'kind-b')).toHaveAttribute('data-context-status', 'not_connected')
    await expect(item(page, FEDERATION_ERROR_CONTEXT)).toHaveAttribute(
      'data-context-status',
      'error',
    )
  })

  test('the error context exposes its message without switching to it (FR-004)', async ({
    page,
  }) => {
    await open(page, 'manual')
    await emit(page, federationContexts)
    await emit(page, federationStatuses)
    const cmdsBefore = await sent(page)
    expect(cmdsBefore.filter((c) => c.type === 'SetContext')).toHaveLength(0)
    await expect(
      item(page, FEDERATION_ERROR_CONTEXT).getByTestId('federation-context-error'),
    ).toHaveText(FEDERATION_ERROR_MESSAGE)
  })

  test('clicking a different context sends SetContext and updates aria-pressed/status on both items (FR-003)', async ({
    page,
  }) => {
    await open(page, 'manual')
    await emit(page, federationContexts)
    await emit(page, federationStatuses)
    await expect(item(page, 'kind-a')).toHaveAttribute('aria-pressed', 'true')
    await expect(item(page, 'kind-b')).toHaveAttribute('aria-pressed', 'false')

    await item(page, 'kind-b').click()

    const cmds = await sent(page)
    expect(cmds.filter((c) => c.type === 'SetContext')).toEqual([
      { type: 'SetContext', data: { context: 'kind-b' } },
    ])

    // The fake backend (manual scenario) sends no automatic reply; the switcher tracks the
    // store's activeContext directly, so simulate the backend's own ContextChanged + status
    // bracketing the real handler emits around a SetContext.
    await emit(
      page,
      batch([
        system(20, { type: 'ContextChanged', data: { current: 'kind-b' } }),
        system(21, {
          type: 'ContextStatusUpdated',
          data: { context: 'kind-b', status: 'Connected', error: null },
        }),
      ]),
    )

    await expect(item(page, 'kind-a')).toHaveAttribute('aria-pressed', 'false')
    await expect(item(page, 'kind-b')).toHaveAttribute('aria-pressed', 'true')
    await expect(item(page, 'kind-b')).toHaveAttribute('data-context-status', 'connected')
  })

  test('re-emitting ContextsUpdated with an added context updates the strip without a reload (FR-005)', async ({
    page,
  }) => {
    await open(page, 'manual')
    await emit(page, federationContexts)
    await emit(page, federationStatuses)
    await expect(switcher(page).getByTestId('federation-context-item')).toHaveCount(3)

    await emit(
      page,
      batch([
        system(30, {
          type: 'ContextsUpdated',
          data: { contexts: ['kind-a', 'kind-b', FEDERATION_ERROR_CONTEXT, 'kind-d'], current: 'kind-a' },
        }),
      ]),
    )
    await expect(switcher(page).getByTestId('federation-context-item')).toHaveCount(4)
    await expect(item(page, 'kind-d')).toHaveAttribute('data-context-status', 'not_connected')
  })
})
