/**
 * US2 (010-container-image-inspection): private-registry auth-fallback outcomes as they reach
 * the UI. The backend does the actual secret-fallback/probe work (src/backend/images.rs, unit-
 * tested with wiremock); here we only verify each `ImageInspectError` kind renders as its own
 * distinct, labeled state rather than a generic failure (FR-009).
 */
import { expect, type Page, test } from '@playwright/test'

import type { ImageInspectError } from '../../src/protocol/types'
import { DETAIL_POD } from '../fixtures'
import { batch, nsRoute, read, SECTIONS } from '../fixtures/builders'
import { emit, inspector, open, row } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`
const container = DETAIL_POD.name.split('-')[0] ?? 'app'

async function openImageTab(page: Page) {
  await open(page)
  await row(page, key(DETAIL_POD)).click()
  const insp = inspector(page)
  await expect(insp).toBeVisible()
  await insp.getByRole('tab', { name: 'Image' }).click()
  return insp
}

const CASES: { kind: ImageInspectError['kind']; detail: string }[] = [
  { kind: 'RegistryUnreachable', detail: 'connection to registry.example.com timed out' },
  { kind: 'ImageNotFound', detail: 'manifest for tag 1.4.2 not found' },
  { kind: 'AuthenticationFailed', detail: 'imagePullSecrets did not authorize this pull' },
  { kind: 'AuthMethodUnsupported', detail: 'registry requires an interactive OIDC challenge' },
  { kind: 'Other', detail: 'unexpected registry response' },
]

test.describe('US2 Image tab auth/error states', () => {
  for (const { kind, detail } of CASES) {
    test(`renders a distinct state for ${kind}`, async ({ page }) => {
      const insp = await openImageTab(page)
      const route = nsRoute('kind-a', SECTIONS.pods, DETAIL_POD.namespace)
      await emit(
        page,
        batch([
          read(300, route, {
            type: 'ContainerImageInspectFailed',
            data: {
              namespace: DETAIL_POD.namespace,
              pod: DETAIL_POD.name,
              container,
              error: { kind, detail },
            },
          }),
        ]),
      )
      const state = insp.getByTestId(`image-inspect-error-${kind}`)
      await expect(state).toBeVisible()
      await expect(state).toContainText(detail)
    })
  }
})
