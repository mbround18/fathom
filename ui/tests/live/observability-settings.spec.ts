/**
 * LIVE (039): Grafana settings persist in the real backend and hydrate after a reload. Touches
 * only local settings (~/.fathom/integrations.json plus one credential-store entry for the
 * bridge's context), never the cluster. The spec clears the key it stored when it finishes.
 *
 * The bridge is pinned to one context (SetContext to another is refused), so "switch context"
 * here re-selects the pinned context through the real picker and checks the settings are
 * re-requested and shown again for it; cross-context isolation is covered in fake mode.
 */
import { chooseScope, expectScope, navigateTo } from '../e2e/helpers'

import { expect, test } from './helpers'

const URL_TEXT = 'https://grafana.e2e.invalid'
const KEY = 'live-e2e-secret-key-039'

test('Grafana URL and key persist, hydrate after reload, and the key never comes back', async ({
  page,
  app,
}) => {
  const received: string[] = []
  page.on('websocket', (ws) => {
    ws.on('framereceived', (f) => received.push(String(f.payload)))
  })

  await app.open()
  await navigateTo(page, 'Grafana', 'Integrations')
  await page.getByLabel('Base URL').fill(URL_TEXT)
  await page.getByLabel('API key').fill(KEY)
  await page.getByRole('button', { name: 'Save' }).click()
  await expect(page.getByTestId('grafana-configured')).toContainText('Configured')
  // The devbridge runs with an in-memory secret store (FATHOM_DEV_MEMORY_SECRETS, debug only).
  await expect(page.getByTestId('grafana-key-state')).toHaveText('Key set')
  const keyState = 'Key set'

  // Reload: a fresh page hydrates from the backend, the key input stays empty.
  await app.open()
  await navigateTo(page, 'Grafana', 'Integrations')
  await expect(page.getByLabel('Base URL')).toHaveValue(URL_TEXT)
  await expect(page.getByLabel('API key')).toHaveValue('')
  await expect(page.getByTestId('grafana-key-state')).toHaveText(keyState)

  // Re-selecting the context re-requests and re-shows the same settings.
  await chooseScope(page, 'Context', app.context)
  await expectScope(page, 'Context', app.context)
  await expect(page.getByLabel('Base URL')).toHaveValue(URL_TEXT)
  await expect(page.getByTestId('grafana-key-state')).toHaveText(keyState)

  expect(await page.content()).not.toContain(KEY)
  expect(received.filter((f) => f.includes(KEY))).toEqual([])

  await page.getByRole('button', { name: 'Clear key' }).click()
  await expect(page.getByTestId('grafana-key-state')).toHaveText('Key not set')
})
