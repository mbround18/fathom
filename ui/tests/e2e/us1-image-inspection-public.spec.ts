/**
 * US1 (010-container-image-inspection): the Image tab against the fake backend. Opening the tab
 * sends `InspectContainerImage`, a `ContainerImageInspected` reply renders manifest/layer/base
 * image details, and re-opening the tab for the same container does not re-send the command
 * (cached result served straight from the store).
 *
 * All testids asserted here are `image-inspect-` prefixed, isolated from 008-pod-lifecycle-
 * diagnostics' `pod-lifecycle-` prefixed testids on the same Inspector page.
 */
import { expect, test } from '@playwright/test'

import type { ImageInspection } from '../../src/protocol/types'
import { DETAIL_POD } from '../fixtures'
import { batch, nsRoute, read, SECTIONS } from '../fixtures/builders'
import { clearSent, emit, inspector, open, row, sent } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`
const container = DETAIL_POD.name.split('-')[0] ?? 'app'

const INSPECTION: ImageInspection = {
  image_ref: {
    registry: 'registry.example.com',
    repository: container,
    tag: '1.4.2',
    digest: 'sha256:' + 'a'.repeat(64),
  },
  total_size_bytes: 42 * 1024 * 1024,
  layers: [
    {
      digest: 'sha256:' + 'b'.repeat(64),
      size_bytes: 20 * 1024 * 1024,
      media_type: 'application/vnd.oci.image.layer.v1.tar+gzip',
      command: 'RUN apt-get update',
      empty_layer: false,
    },
    {
      digest: 'sha256:' + 'c'.repeat(64),
      size_bytes: 22 * 1024 * 1024,
      media_type: 'application/vnd.oci.image.layer.v1.tar+gzip',
      command: 'COPY . /app',
      empty_layer: false,
    },
  ],
  created: '2026-09-01T00:00:00Z',
  base_image: { label: 'debian:bookworm-slim', confidence: 'high' },
}

test.describe('US1 Image tab', () => {
  test('opening Image sends InspectContainerImage and renders the reply', async ({ page }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()

    await clearSent(page)
    await insp.getByRole('tab', { name: 'Image' }).click()
    await expect(insp.getByRole('tab', { name: 'Image' })).toHaveAttribute('aria-selected', 'true')

    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toEqual(expect.arrayContaining(['InspectContainerImage']))
    const cmd = (await sent(page)).find((c) => c.type === 'InspectContainerImage')
    expect(cmd).toEqual({
      type: 'InspectContainerImage',
      data: { namespace: DETAIL_POD.namespace, pod: DETAIL_POD.name, container },
    })

    const route = nsRoute('kind-a', SECTIONS.pods, DETAIL_POD.namespace)
    await emit(
      page,
      batch([
        read(300, route, {
          type: 'ContainerImageInspected',
          data: {
            namespace: DETAIL_POD.namespace,
            pod: DETAIL_POD.name,
            container,
            inspection: INSPECTION,
          },
        }),
      ]),
    )

    const panel = insp.getByTestId('image-inspect-panel')
    await expect(panel.getByTestId('image-inspect-ref')).toContainText('registry.example.com')
    await expect(panel.getByTestId('image-inspect-ref')).toContainText(container)
    await expect(panel.getByTestId('image-inspect-base-image')).toContainText(
      'debian:bookworm-slim',
    )
    await expect(panel.getByTestId('image-inspect-layers')).toContainText('RUN apt-get update')
    await expect(panel.getByTestId('image-inspect-layers')).toContainText('COPY . /app')

    // Re-opening the same tab/container is served from the cache: no second command is sent.
    await insp.getByRole('tab', { name: 'Details' }).click()
    await clearSent(page)
    await insp.getByRole('tab', { name: 'Image' }).click()
    await expect(panel.getByTestId('image-inspect-ref')).toContainText('registry.example.com')
    expect((await sent(page)).map((c) => c.type)).not.toContain('InspectContainerImage')
  })
})
