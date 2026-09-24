/**
 * US1 (YAML edit + apply, T012-T016): the YAML tab's edit -> review diff -> confirm -> apply
 * flow against the fake backend (FR-004, FR-005, FR-006, FR-007).
 *
 *  - T012 happy path: edit, review, confirm, apply succeeds (`ResourceApplied` + `CommandFinished`).
 *  - T013: an identity-field edit (metadata.name) is rejected by the backend (FR-005) — the
 *    command still carries the session's original identity, only the backend's rejection message
 *    (no "conflict:"/"not-found:" prefix) classifies as a generic error.
 *  - T014: a stale-`resourceVersion` conflict (409, "conflict: ...") is classified distinctly and
 *    preserves the edit buffer.
 *  - T015 (NEW): a deleted-mid-edit (404, "not-found: ...") is classified distinctly from T014,
 *    never conflated, and also preserves the edit buffer.
 *  - T016: an unsaved (dirty) edit warns before leaving the YAML tab.
 */
import { expect, test, type Page } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { batch, nsRoute, stream } from '../fixtures/builders'
import {
  clearSent,
  confirmDialog,
  emit,
  finish,
  inspector,
  nextRequestId,
  open,
  row,
  sent,
} from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

async function openYamlTab(page: Page) {
  await open(page)
  await row(page, key(DETAIL_POD)).click()
  const insp = inspector(page)
  await expect(insp).toBeVisible()
  await insp.getByRole('tab', { name: 'YAML' }).click()
  const textarea = insp.getByTestId('yaml-edit-textarea')
  await expect(textarea).toBeVisible()
  return { insp, textarea }
}

test.describe('US1 YAML edit + apply', () => {
  test('T012: edit, review diff, confirm, apply succeeds', async ({ page }) => {
    const { insp, textarea } = await openYamlTab(page)

    const original = await textarea.inputValue()
    expect(original).toContain(`name: ${DETAIL_POD.name}`)
    const edited = original.replace('labels:', 'labels:\n  team: platform')
    await textarea.fill(edited)

    await insp.getByTestId('yaml-review').click()
    const diff = insp.getByTestId('yaml-diff')
    await expect(diff).toBeVisible()
    await expect(insp.getByTestId('diff-added')).toContainText('team: platform')

    await clearSent(page)
    const id = await nextRequestId(page)
    await insp.getByTestId('yaml-apply').click()

    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await dialog.getByTestId('confirm-submit').click()

    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toEqual(expect.arrayContaining(['ApplyResourceYaml']))
    const cmd = (await sent(page)).find((c) => c.type === 'ApplyResourceYaml')
    expect(cmd).toMatchObject({
      type: 'ApplyResourceYaml',
      data: {
        api_version: 'v1',
        kind: 'Pod',
        namespace: DETAIL_POD.namespace,
        name: DETAIL_POD.name,
      },
    })

    await expect(insp.getByTestId('yaml-apply-status')).toHaveAttribute('data-status', 'applying')

    const route = nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace)
    await emit(
      page,
      batch([
        stream(900, route, {
          type: 'ResourceApplied',
          data: { request_id: String(id), resource_version: '999' },
        }),
      ]),
    )
    await finish(page, id, true)

    await expect(insp.getByTestId('yaml-apply-status')).toHaveAttribute('data-status', 'applied')
    await expect(insp.getByTestId('yaml-discard')).toHaveCount(0)
  })

  test('T013: an identity-field edit is rejected (not conflated with conflict/deleted)', async ({
    page,
  }) => {
    const { insp, textarea } = await openYamlTab(page)
    const original = await textarea.inputValue()
    const edited = original.replace(`name: ${DETAIL_POD.name}`, 'name: renamed-pod')
    await textarea.fill(edited)

    await insp.getByTestId('yaml-review').click()
    await expect(insp.getByTestId('yaml-diff')).toBeVisible()

    const id = await nextRequestId(page)
    await insp.getByTestId('yaml-apply').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()

    const rejection =
      'identity fields cannot change (kind, apiVersion, metadata.name, metadata.namespace ' +
      `target the one resource that was opened): metadata.name changed from "${DETAIL_POD.name}" to "renamed-pod"`
    await finish(page, id, false, rejection)

    const status = insp.getByTestId('yaml-apply-status')
    await expect(status).toHaveAttribute('data-status', 'error')
    await expect(insp.getByTestId('yaml-apply-error')).toHaveText(rejection)
    // Not classified as conflict or deleted (spec analyze finding C2).
    await expect(status).not.toHaveAttribute('data-status', 'conflict')
    await expect(status).not.toHaveAttribute('data-status', 'deleted')
    // The edit buffer (with the operator's identity edit) is preserved, not discarded.
    await expect(insp.getByTestId('yaml-edit-textarea')).toHaveValue(edited)
  })

  test('T014: a stale-resourceVersion conflict is classified distinctly and preserves the buffer', async ({
    page,
  }) => {
    const { insp, textarea } = await openYamlTab(page)
    const original = await textarea.inputValue()
    const edited = original.replace('labels:', 'labels:\n  team: platform')
    await textarea.fill(edited)

    await insp.getByTestId('yaml-review').click()
    const id = await nextRequestId(page)
    await insp.getByTestId('yaml-apply').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()

    await finish(
      page,
      id,
      false,
      'conflict: Operation cannot be fulfilled: the object has been modified',
    )

    const status = insp.getByTestId('yaml-apply-status')
    await expect(status).toHaveAttribute('data-status', 'conflict')
    await expect(insp.getByTestId('yaml-apply-error')).toContainText(
      'Operation cannot be fulfilled',
    )
    await expect(insp.getByTestId('yaml-reload')).toBeVisible()

    // Buffer preserved: the operator's edit is still in the textarea, not discarded.
    await expect(insp.getByTestId('yaml-edit-textarea')).toHaveValue(edited)
  })

  test('T015: a deleted-mid-edit is classified distinctly from conflict and preserves the buffer', async ({
    page,
  }) => {
    const { insp, textarea } = await openYamlTab(page)
    const original = await textarea.inputValue()
    const edited = original.replace('labels:', 'labels:\n  team: platform')
    await textarea.fill(edited)

    await insp.getByTestId('yaml-review').click()
    const id = await nextRequestId(page)
    await insp.getByTestId('yaml-apply').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()

    await finish(page, id, false, `not-found: pods "${DETAIL_POD.name}" not found`)

    const status = insp.getByTestId('yaml-apply-status')
    await expect(status).toHaveAttribute('data-status', 'deleted')
    await expect(status).not.toHaveAttribute('data-status', 'conflict')
    await expect(insp.getByTestId('yaml-apply-error')).toContainText('not found')
    await expect(insp.getByTestId('yaml-reload')).toBeVisible()

    await expect(insp.getByTestId('yaml-edit-textarea')).toHaveValue(edited)
  })

  test('T016: an unsaved edit warns before leaving the YAML tab', async ({ page }) => {
    const { insp, textarea } = await openYamlTab(page)
    const original = await textarea.inputValue()
    await textarea.fill(original.replace('labels:', 'labels:\n  team: platform'))
    await expect(insp.getByTestId('yaml-discard')).toBeVisible() // dirty

    let dialogSeen = false
    page.once('dialog', (d) => {
      dialogSeen = true
      void d.dismiss()
    })
    await insp.getByRole('tab', { name: 'Details' }).click()
    await expect.poll(() => dialogSeen).toBe(true)
    // Dismissing the confirm keeps the operator on the YAML tab with the edit intact.
    await expect(insp.getByTestId('yaml-edit-textarea')).toBeVisible()
    await expect(insp.getByTestId('yaml-edit-textarea')).toHaveValue(
      original.replace('labels:', 'labels:\n  team: platform'),
    )

    page.once('dialog', (d) => void d.accept())
    await insp.getByRole('tab', { name: 'Details' }).click()
    await expect(insp.getByRole('tab', { name: 'Details' })).toHaveAttribute(
      'aria-selected',
      'true',
    )
  })
})
