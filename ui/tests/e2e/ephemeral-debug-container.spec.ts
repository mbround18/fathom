/**
 * 025-ephemeral-debug-containers: "Add debug container" on a pod shows the irreversibility
 * warning, sends exactly one `AddEphemeralContainer` on confirm, and the new ephemeral container
 * appears in the pod's container list once the backend reports it.
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { podDetails } from '../fixtures/gen-pods'
import { podDetailWithDebugContainer } from '../fixtures/batches'
import {
  actionToast,
  clearSent,
  emit,
  finish,
  inspector,
  nextRequestId,
  open,
  row,
  sent,
} from './helpers'

const KEY = `${DETAIL_POD.namespace}/${DETAIL_POD.name}`

test('add debug container: warning, one command, container appears', async ({ page }) => {
  await open(page)
  await row(page, KEY).click()
  await expect(inspector(page)).toBeVisible()
  await expect(inspector(page)).toContainText('Containers (1)')

  await inspector(page).getByTestId('inspector-actions').click()
  await page.getByTestId('action-debug').click()

  const dialog = page.getByTestId('debug-dialog')
  await expect(dialog).toBeVisible()
  await expect(dialog.getByTestId('debug-warning')).toContainText(
    'Ephemeral containers cannot be removed once added and will persist for the life of the pod.',
  )
  await expect(dialog.getByTestId('debug-submit')).toBeDisabled()
  await dialog.getByTestId('debug-image').fill('busybox')
  await expect(dialog.getByTestId('debug-summary')).toContainText(KEY)
  await expect(dialog.getByTestId('debug-summary')).toContainText('busybox')
  await expect(dialog.getByTestId('debug-submit')).toBeEnabled()

  await clearSent(page)
  const id = await nextRequestId(page)
  await dialog.getByTestId('debug-submit').click()
  await expect(dialog).toBeHidden()
  await expect
    .poll(() => sent(page))
    .toEqual([
      {
        type: 'AddEphemeralContainer',
        data: {
          namespace: DETAIL_POD.namespace,
          pod: DETAIL_POD.name,
          target_container: podDetails(DETAIL_POD).containers[0],
          image: 'busybox',
          ephemeral_container_name: null,
        },
      },
    ])

  await finish(page, id, true)
  await expect(actionToast(page, 'success')).toContainText('Added debug container to')
  await emit(page, podDetailWithDebugContainer('kind-a', DETAIL_POD, 'debugger-abc', 'busybox'))
  await expect(inspector(page)).toContainText('Debug containers (1)')
  await expect(inspector(page)).toContainText('debugger-abc (waiting)')
})
