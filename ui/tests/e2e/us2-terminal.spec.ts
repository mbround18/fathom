/**
 * T018 (US2 Phase 4): the Terminal tab against the fake backend. Opening Terminal on a Running
 * pod sends `StartPodExec`, keystrokes send `SendPodExecInput`, injected `PodExecOutputChunk`
 * events render in the terminal, and switching to a different pod's Terminal stops the first
 * session before starting the second (FR-007..FR-013, FR-012 singleton session).
 */
import { expect, test, type Page } from '@playwright/test'

import { DETAIL_POD, PODS_500 } from '../fixtures'
import { batch, nsRoute, read, stream } from '../fixtures/builders'
import { clearSent, emit, inspector, open, row, sent, sentTypes } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`
/** 024: the gate — answer the access pre-check as allowed, then confirm. */
async function passGate(page: Page, pod: { namespace: string; name: string }, seq: number) {
  await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['CheckPodExecAccess']))
  await emit(
    page,
    batch([
      read(seq, nsRoute('kind-a', 'workloads/pods', pod.namespace), {
        type: 'PodExecAccessChecked',
        data: {
          namespace: pod.namespace,
          name: pod.name,
          container: null,
          allowed: true,
          reason: null,
        },
      }),
    ]),
  )
  await page.getByTestId('confirm-submit').click()
}

const SECOND_POD = PODS_500[3]
if (!SECOND_POD) throw new Error('fixture')

function encodeChunk(text: string): string {
  return Buffer.from(text, 'utf-8').toString('base64')
}

test.describe('US2 Terminal tab', () => {
  test('opening Terminal starts a session, input/output round-trip, switching pods stops the first', async ({
    page,
  }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect(insp.getByRole('heading', { name: DETAIL_POD.name })).toBeVisible()

    await clearSent(page)
    await insp.getByRole('tab', { name: 'Terminal' }).click()
    await expect(insp.getByRole('tab', { name: 'Terminal' })).toHaveAttribute(
      'aria-selected',
      'true',
    )

    await passGate(page, DETAIL_POD, 290)
    await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['StartPodExec']))

    const route = nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace)

    // FR-007/FR-008: single-container pod, no picker — the session attaches immediately.
    await emit(page, batch([stream(300, route, { type: 'PodExecReady', data: { stream_id: 1 } })]))

    const xterm = insp.getByTestId('pod-terminal-xterm')
    await expect(xterm).toBeVisible()

    // FR-009/FR-011: keystrokes are relayed as SendPodExecInput.
    await xterm.click()
    await page.keyboard.type('ls')
    await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['SendPodExecInput']))

    // Output chunks render into the terminal (screenReaderMode gives an accessible text tree).
    await emit(
      page,
      batch([
        stream(301, route, {
          type: 'PodExecOutputChunk',
          data: { stream_id: 1, data: encodeChunk('hello from the pod\r\n') },
        }),
      ]),
    )
    await expect(xterm).toContainText('hello from the pod')

    // FR-012: switching to a different pod's Terminal stops the first session before starting
    // the second (the Inspector remounts on target change, unmount-before-mount).
    await clearSent(page)
    // Phone shows one pane at a time: the list is hidden behind the inspector until Back.
    if (test.info().project.metadata.widthClass === 'phone') {
      await page.getByRole('button', { name: 'Back' }).click()
    }
    await row(page, key(SECOND_POD)).click()

    const insp2 = inspector(page)
    await expect(insp2).toBeVisible()
    await expect(insp2.getByRole('heading', { name: SECOND_POD.name })).toBeVisible()

    await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['StopPodExec']))

    await insp2.getByRole('tab', { name: 'Terminal' }).click()
    await passGate(page, SECOND_POD, 310)
    await expect
      .poll(() => sentTypes(page))
      .toEqual(expect.arrayContaining(['StopPodExec', 'StartPodExec']))

    const types = await sentTypes(page)
    const stopIdx = types.indexOf('StopPodExec')
    const startIdx = types.lastIndexOf('StartPodExec')
    expect(stopIdx).toBeGreaterThanOrEqual(0)
    expect(startIdx).toBeGreaterThan(stopIdx)

    const start2 = (await sent(page)).find((c) => c.type === 'StartPodExec')
    expect(start2).toMatchObject({
      type: 'StartPodExec',
      data: { namespace: SECOND_POD.namespace, name: SECOND_POD.name },
    })
  })

  test('an attach failure (e.g. RBAC-denied) shows the verbatim backend reason', async ({
    page,
  }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()

    await clearSent(page)
    await insp.getByRole('tab', { name: 'Terminal' }).click()
    await passGate(page, DETAIL_POD, 290)
    await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['StartPodExec']))

    const route = nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace)
    await emit(
      page,
      batch([
        stream(300, route, {
          type: 'StreamClosed',
          data: {
            stream_id: 1,
            reason:
              'Permission denied: exec into this pod is forbidden (check RBAC for pods/exec).',
          },
        }),
      ]),
    )

    const errorPane = insp.locator('[data-pane-state="terminal-error"]')
    await expect(errorPane).toBeVisible()
    await expect(errorPane).toContainText(
      'Permission denied: exec into this pod is forbidden (check RBAC for pods/exec).',
    )
  })
})
