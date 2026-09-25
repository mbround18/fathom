/**
 * 024-pod-exec-terminal: the Terminal tab's pre-session gate. A proactive `CheckPodExecAccess`
 * runs first; a denied result shows a permission message with no confirmation and no
 * `StartPodExec`; an allowed result requires explicit confirmation before `StartPodExec`.
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { batch, nsRoute, read, stream } from '../fixtures/builders'
import { clearSent, emit, inspector, open, row, sent, sentTypes } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

function checked(allowed: boolean, reason: string | null) {
  return read(400, nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace), {
    type: 'PodExecAccessChecked',
    data: {
      namespace: DETAIL_POD.namespace,
      name: DETAIL_POD.name,
      container: null,
      allowed,
      reason,
    },
  })
}

test.describe('Pod exec session gate', () => {
  test('allowed: confirmation precedes StartPodExec; cancel sends nothing', async ({ page }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()
    const insp = inspector(page)
    await expect(insp).toBeVisible()

    await clearSent(page)
    await insp.getByRole('tab', { name: 'Terminal' }).click()
    await expect.poll(() => sentTypes(page)).toContain('CheckPodExecAccess')
    expect(await sentTypes(page)).not.toContain('StartPodExec')

    await emit(page, batch([checked(true, null)]))
    const dialog = page.getByTestId('confirm-dialog')
    await expect(dialog).toBeVisible()
    await expect(dialog).toContainText(DETAIL_POD.name)
    expect(await sentTypes(page)).not.toContain('StartPodExec')

    // Cancel: nothing is started.
    await page.getByTestId('confirm-cancel').click()
    await expect(dialog).toBeHidden()
    expect(await sentTypes(page)).not.toContain('StartPodExec')

    // Re-open: confirmation is required again; confirming starts the session.
    await insp.getByRole('button', { name: 'Open Terminal' }).click()
    await expect
      .poll(async () => (await sentTypes(page)).filter((t) => t === 'CheckPodExecAccess').length)
      .toBe(2)
    await emit(page, batch([checked(true, null)]))
    await expect(dialog).toBeVisible()
    await page.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('StartPodExec')

    const route = nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace)
    await emit(
      page,
      batch([
        stream(401, route, { type: 'PodExecReady', data: { stream_id: 1 } }),
        stream(402, route, {
          type: 'PodExecOutputChunk',
          data: { stream_id: 1, data: Buffer.from('hello gate\r\n').toString('base64') },
        }),
      ]),
    )
    await expect(insp.getByTestId('pod-terminal-xterm')).toContainText('hello gate')
  })

  test('denied: permission message, no confirmation, no StartPodExec', async ({ page }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()
    const insp = inspector(page)
    await expect(insp).toBeVisible()

    await clearSent(page)
    await insp.getByRole('tab', { name: 'Terminal' }).click()
    await expect.poll(() => sentTypes(page)).toContain('CheckPodExecAccess')

    await emit(page, batch([checked(false, 'user cannot create pods/exec in this namespace')]))
    const denied = insp.locator('[data-pane-state="terminal-denied"]')
    await expect(denied).toBeVisible()
    await expect(denied).toContainText('user cannot create pods/exec in this namespace')
    await expect(page.getByTestId('confirm-dialog')).toHaveCount(0)
    expect((await sent(page)).some((c) => c.type === 'StartPodExec')).toBe(false)
  })
})
