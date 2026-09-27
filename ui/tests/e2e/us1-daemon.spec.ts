/**
 * 036-incluster-daemon (T024/T025/T051): status chip states, preference toggle, "While you were
 * away" timeline, truncation banner, daemon-fed topology with fallback, sparkline truncation.
 * Runs against the scripted fake backend; the daemon itself is simulated by emitting events.
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import {
  daemonBatch,
  daemonStatus,
  entry,
  metricsBatch,
  sampleTopology,
  topologyBatch,
} from '../fixtures/daemon'
import {
  clearSent,
  emit,
  expectNoHorizontalScroll,
  open,
  openTopologySection,
  sent,
} from './helpers'
import type { DaemonState } from '../../src/protocol/types'

const stacked = (i: TestInfo): boolean =>
  i.project.name.endsWith('-phone') || i.project.name.endsWith('-tablet')

const chip = (page: Page) => page.getByTestId('daemon-chip')

async function openDialog(page: Page) {
  await chip(page).click()
  await expect(page.getByTestId('daemon-dialog')).toBeVisible()
}

test.describe('daemon status chip', () => {
  test('defaults to off, offers a copy-only install hint, sends no daemon requests', async ({
    page,
  }) => {
    await open(page)
    await expect(chip(page)).toHaveAttribute('data-daemon-state', 'off')
    await openDialog(page)
    await expect(page.getByTestId('daemon-install-hint')).toContainText('kubectl')
    await expect(page.getByTestId('daemon-toggle')).not.toBeChecked()
    const daemonCmds = (await sent(page)).filter((c) => /Daemon/.test(c.type))
    expect(daemonCmds).toEqual([])
  })

  test('toggling on sends SetDaemonPreference auto', async ({ page }) => {
    await open(page)
    await openDialog(page)
    await clearSent(page)
    await page.getByTestId('daemon-toggle').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'SetDaemonPreference'))
      .toMatchObject({ data: { context: 'kind-a', mode: 'auto' } })
  })

  const states: [DaemonState, string | null][] = [
    ['notInstalled', null],
    ['unavailable', 'connection refused'],
    ['noAccess', 'forbidden: cannot get daemon.fathom.dev'],
    ['versionMismatch', 'daemon protocol 2 is not supported'],
    ['available', null],
  ]
  for (const [state, detail] of states) {
    test(`shows the ${state} state`, async ({ page }) => {
      await open(page)
      await emit(page, daemonStatus(state, detail))
      await expect(chip(page)).toHaveAttribute('data-daemon-state', state)
      await openDialog(page)
      await expect(page.getByTestId('daemon-toggle')).toBeChecked()
      if (detail) await expect(page.getByTestId('daemon-error')).toContainText(detail)
      if (state === 'available') await expect(page.getByTestId('daemon-capabilities')).toBeVisible()
      await page.keyboard.press('Escape')
      await expectNoHorizontalScroll(page)
    })
  }
})

// Laptop (1280-1919) has neither the stacked tab strip nor the dock, so tool panes are not
// reachable there (pre-existing: us1-topology-overview fails at that width too).
const noToolPanes = (i: TestInfo): boolean => i.project.name.endsWith('-laptop')

test.describe('timeline', () => {
  test('shows derived vs raw origin, verbatim message and truncation banner', async ({
    page,
  }, info) => {
    test.skip(noToolPanes(info), 'no tool-pane surface at laptop width')
    await open(page)
    await emit(page, daemonStatus('available'))
    await emit(
      page,
      daemonBatch(
        1,
        [
          entry(1),
          entry(2, {
            origin: 'raw',
            reason: 'BackOff',
            message: 'Back-off restarting failed container',
            severity: 'warning',
          }),
        ],
        { oldestSeq: 1, oldestTime: '2026-09-24T09:00:00Z', dropped: 12, reason: 'cursorTooOld' },
      ),
    )
    if (stacked(info)) {
      await openDialog(page)
      await page.getByTestId('daemon-open-timeline').click()
    } else {
      await page.getByRole('tab', { name: 'While you were away' }).click()
    }
    const tl = page.getByTestId('daemon-timeline').first()
    await expect(tl).toBeVisible()
    await expect(page.getByTestId('daemon-truncation-banner').first()).toBeVisible()
    const entries = page.getByTestId('daemon-timeline-entry')
    await expect(entries).toHaveCount(2)
    await expect(
      entries.filter({ hasText: 'BackOff' }).getByTestId('daemon-origin-badge'),
    ).toHaveText('Raw')
    await expect(
      entries.filter({ hasText: 'ProgressDeadlineExceeded' }).getByTestId('daemon-origin-badge'),
    ).toHaveText('Derived')
    await expect(tl.getByText('Back-off restarting failed container')).toBeVisible()
    await expectNoHorizontalScroll(page)
  })
})

test.describe('daemon topology and trends', () => {
  async function openTopology(page: Page, info: TestInfo) {
    await open(page)
    await emit(page, daemonStatus('available'))
    await emit(page, topologyBatch(1, sampleTopology()))
    if (stacked(info)) {
      await page
        .getByRole('navigation', { name: 'Views' })
        .getByRole('button', { name: 'Topology' })
        .click()
    } else {
      // 043 US6: Topology left the dock for the navigator's Cluster group.
      await openTopologySection(page)
    }
  }

  test('renders the daemon graph and per-object trend with truncation marker', async ({
    page,
  }, info) => {
    test.skip(noToolPanes(info), 'no tool-pane surface at laptop width')
    await openTopology(page, info)
    await expect(page.getByTestId('daemon-topology')).toBeVisible()
    await expect(page.getByTestId('daemon-topology-node')).toHaveCount(3)
    await page.locator('[data-uid="pod1"]').click()
    await expect(page.getByTestId('daemon-topology-relations')).toContainText('web')
    await expect
      .poll(async () => (await sent(page)).some((c) => c.type === 'RequestDaemonMetrics'))
      .toBe(true)
    await emit(
      page,
      metricsBatch(1, [
        {
          key: { objectUid: 'pod1', metric: 'cpu' },
          samples: [
            { time: '2026-09-24T10:00:00Z', value: 1 },
            { time: '2026-09-24T10:01:00Z', value: 2 },
          ],
          truncatedOldestTime: '2026-09-24T09:30:00Z',
        },
      ]),
    )
    await expect(page.getByTestId('daemon-sparkline').first()).toBeVisible()
    await expect(page.getByTestId('daemon-sparkline-truncation').first()).toBeAttached()
    await expect(page.getByTestId('daemon-sparkline-note').first()).toBeVisible()
    await expectNoHorizontalScroll(page)
  })

  test('falls back to the client-built topology when the daemon goes away', async ({
    page,
  }, info) => {
    test.skip(noToolPanes(info), 'no tool-pane surface at laptop width')
    await openTopology(page, info)
    await expect(page.getByTestId('daemon-topology')).toBeVisible()
    await emit(page, daemonStatus('unavailable', 'connection refused'))
    await expect(page.getByTestId('daemon-topology')).toHaveCount(0)
    await expect(page.getByTestId('topology-canvas')).toBeVisible()
  })
  test('draws the cytoscape graph at wide widths and keeps the list as the alternative', async ({
    page,
  }, info) => {
    test.skip(noToolPanes(info), 'no tool-pane surface at laptop width')
    await openTopology(page, info)
    const graph = page.getByTestId('daemon-topology-graph')
    if (stacked(info)) {
      await expect(graph).toHaveCount(0)
    } else {
      await expect(graph).toHaveAttribute('data-node-count', '3')
      expect((await graph.boundingBox())?.height).toBeGreaterThan(100)
    }
    await expect(page.getByTestId('daemon-topology-node')).toHaveCount(3)
    await emit(
      page,
      topologyBatch(1, {
        seq: 2,
        added: [{ uid: 'pod2', kind: 'Pod', namespace: 'shop', name: 'web-def', health: 'error' }],
        removed: [],
        edges: { added: [{ from: 'dep1', to: 'pod2', relation: 'owns' }], removed: [] },
      }),
    )
    await expect(page.getByTestId('daemon-topology-node')).toHaveCount(4)
    if (!stacked(info)) await expect(graph).toHaveAttribute('data-node-count', '4')
  })
})
