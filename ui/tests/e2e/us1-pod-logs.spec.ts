/**
 * T011 (US1 Phase 3): the Logs tab against the fake backend. Selecting a pod and opening Logs
 * sends `StartPodLogs`, injected `PodLogsChunk` events render as lines, and leaving the tab
 * (closing the inspector) sends `StopPodLogStream` for that stream's id (FR-002, FR-004, FR-005,
 * FR-006; feedback #2: every logs view owns its stream under a client-chosen id).
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { batch, nsRoute, stream } from '../fixtures/builders'
import { clearSent, emit, inspector, open, row, sent } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

test.describe('US1 Logs tab', () => {
  test('opening Logs sends StartPodLogs, chunks render, closing stops that stream', async ({
    page,
  }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect(insp.getByRole('heading', { name: DETAIL_POD.name })).toBeVisible()

    await clearSent(page)
    await insp.getByRole('tab', { name: 'Logs' }).click()
    await expect(insp.getByRole('tab', { name: 'Logs' })).toHaveAttribute('aria-selected', 'true')

    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toEqual(expect.arrayContaining(['StartPodLogs']))
    const start = (await sent(page)).find((c) => c.type === 'StartPodLogs')
    expect(start).toMatchObject({
      type: 'StartPodLogs',
      data: { namespace: DETAIL_POD.namespace, name: DETAIL_POD.name },
    })
    const streamId = start?.type === 'StartPodLogs' ? start.data.stream_id : undefined
    if (streamId === undefined) throw new Error('StartPodLogs carries a client stream id')

    const route = nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace)
    const scroll = insp.getByTestId('pod-logs-scroll')
    // Opened with no output yet: an explicit empty state, not a loading skeleton.
    await emit(
      page,
      batch([stream(199, route, { type: 'PodLogsOpened', data: { stream_id: streamId } })]),
    )
    await expect(insp.locator('[data-pane-state="logs-empty"]')).toHaveText('No log output yet.')

    await emit(
      page,
      batch([
        stream(200, route, {
          type: 'PodLogsChunk',
          data: { stream_id: streamId, lines: ['booting…', 'listening on :8080'] },
        }),
      ]),
    )
    await expect(scroll).toContainText('booting…')
    await expect(scroll).toContainText('listening on :8080')

    // Search narrows; a result shows the whole log at that line.
    await insp.getByRole('searchbox', { name: 'Search logs' }).fill('listening')
    await expect(insp.locator('[data-field="matches"]')).toHaveText('1 of 2')
    await expect(scroll).not.toContainText('booting…')
    await scroll.locator('[data-action="show-in-context"]').click()
    await expect(scroll).toHaveAttribute('data-view', 'context')
    await expect(scroll).toContainText('booting…')
    await expect(scroll.locator('[data-line="1"]')).toHaveAttribute('data-flash', '')
    await insp.getByRole('button', { name: /Back to results/ }).click()
    await insp.getByRole('searchbox', { name: 'Search logs' }).fill('')
    await expect(scroll).toHaveAttribute('data-view', 'all')

    // FR-006: a StreamClosed shows a distinct "stream ended" indicator, not a blank pane.
    await emit(
      page,
      batch([
        stream(201, route, { type: 'StreamClosed', data: { stream_id: streamId, reason: 'EOF' } }),
      ]),
    )
    await expect(insp.locator('[data-pane-state="stream-ended"]')).toBeVisible()
    await expect(insp.locator('[data-pane-state="stream-ended"]')).toContainText('EOF')

    // Leaving the tab (closing the inspector) stops the stream (FR-005).
    await clearSent(page)
    await page.keyboard.press('Escape')
    await expect
      .poll(async () => await sent(page))
      .toEqual(
        expect.arrayContaining([{ type: 'StopPodLogStream', data: { stream_id: streamId } }]),
      )
  })
})
