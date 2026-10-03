/**
 * 048-docker-management US3: reclaim disk without breaking another project.
 *
 * The six acceptance scenarios of the story. Between them one claim, and it is the reason this
 * story has a guard at all: the daemon under this scene is shared with unrelated projects, and a
 * careless prune on it has already cost real work.
 *
 * Three of these are worth reading twice.
 *
 * A prune is never issued as a prune. The preview resolves it into the identities it covers right
 * now and the argv is a removal of exactly those (FR-026), which is what makes it possible to
 * assert that the list on screen *is* the operation — `docker image prune --force` run a second
 * later is a different operation from the one previewed, and this one cannot be.
 *
 * Exclusion is total, not a warning (FR-025). An object any unadopted project can reach is left
 * out of the removal set entirely, and reachability counts every project that references it rather
 * than only the one it declares. A warning the operator can confirm past is exactly how an
 * unrelated project's volume gets deleted.
 *
 * And the runtime is asked again afterwards (FR-023). A removal that reported success over an
 * object the next reading still holds is reported as a discrepancy rather than shown as a green
 * tick, because a runtime that lies is the case the verification exists for.
 */
import { expect, test, type Page } from '@playwright/test'

import { sizeLabel } from '../../src/features/containers/labels'
import { DEFAULT_OBJECTS, volumeInUse } from '../fixtures/containers'
import { open, sentTypes } from './helpers'

/** The group and its first section share the label "Containers", so the click is scoped by row. */
async function openSection(page: Page, label: string): Promise<void> {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav).toBeVisible()
  const toggle = nav.locator('button[aria-expanded]', { hasText: 'Containers' }).first()
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
  await nav.locator('li button', { hasText: label }).first().click()
}

/** The scene, with one of the three object panes on screen. */
async function pane(
  page: Page,
  scenario: Parameters<typeof open>[1],
  label: string,
  objectClass: string,
): Promise<void> {
  await open(page, scenario)
  await expect.poll(async () => await sentTypes(page)).toContain('RefreshContainerRuntimePresence')
  await openSection(page, label)
  await expect(page.getByTestId(`object-pane-${objectClass}`)).toBeVisible()
}

/** Type the confirmation the preview asks for, reading it where the operator reads it. */
async function typeConfirmation(page: Page, text?: string): Promise<string> {
  const required =
    (await page.locator('label[for="operation-confirmation"] .font-mono').textContent()) ?? ''
  await page.getByTestId('operation-confirmation').fill(text ?? required.trim())
  return required.trim()
}

const IMAGES = DEFAULT_OBJECTS.images
const byName = (name: string) => {
  const found = [...IMAGES, ...DEFAULT_OBJECTS.volumes, ...DEFAULT_OBJECTS.networks].find(
    (o) => o.name === name,
  )
  if (!found) throw new Error(`no fixture object named ${name}`)
  return found
}

test.describe('US3 reclaim space', () => {
  /**
   * Scenario 1. Three tag words, not two: `tagged: null` means the runtime did not say, and
   * calling that "untagged" would invite a prune of an image that may well be tagged.
   */
  test('each image states its size, its tag state and what still references it', async ({
    page,
  }) => {
    await pane(page, 'containers', 'Images', 'image')

    for (const image of IMAGES) {
      await expect(page.getByTestId(`object-size-${image.id}`)).toHaveText(
        sizeLabel(image.size_bytes) ?? 'size not reported',
      )
    }
    await expect(page.getByTestId('object-tagged-sha256:aaaa')).toHaveText('tagged')
    await expect(page.getByTestId('object-tagged-sha256:bbbb')).toHaveText('untagged')
    // The one the runtime would not classify is said to be unclassified, in its own row, so it is
    // never read as interchangeable with the ones above it.
    await expect(page.getByTestId('object-tagged-sha256:dddd')).toHaveText('not classified')

    await expect(page.getByTestId('object-references-sha256:aaaa')).toHaveText('nothing uses it')
    await expect(page.getByTestId('object-references-sha256:cccc')).toHaveText(
      'blog-worker (stopped)',
    )
    await expect(page.getByTestId('object-row-sha256:aaaa')).toHaveAttribute('data-in-use', 'false')
    await expect(page.getByTestId('object-row-sha256:cccc')).toHaveAttribute('data-in-use', 'true')
  })

  /**
   * Scenario 2. The list *is* the operation: every id in the argv has a row above it, and every
   * row has a name. Nothing is summarised as a count, which is the state in which this operation
   * must not be issuable — so the assertion is that the two agree, not that a total exists.
   */
  test('a prune names every object it would remove and the space that frees', async ({ page }) => {
    await pane(page, 'containers-adopted', 'Images', 'image')
    await page.getByTestId('object-prune-unused_images').click()

    const preview = page.getByTestId('operation-preview')
    await expect(preview).toBeVisible()
    // Resolved to identities, never handed to the daemon as a prune (FR-026).
    const argv = (await page.getByTestId('operation-command').textContent()) ?? ''
    expect(argv).toContain('docker image rm')
    expect(argv).not.toContain('prune')

    const unused = IMAGES.filter((o) => o.referenced_by.length === 0)
    const rows = page.getByTestId('operation-removals').locator('li')
    await expect(rows).toHaveCount(unused.length)
    // One row per id in the argv, each carrying the object's own name: the operator can read what
    // goes without reading the command, and the two cannot disagree.
    for (const image of unused) {
      expect(argv).toContain(image.id)
      await expect(page.getByTestId(`operation-removal-${image.id}`)).toContainText(image.name)
    }
    const freed = unused.reduce((sum, o) => sum + (o.size_bytes ?? 0), 0)
    await expect(page.getByTestId('operation-total-size')).toContainText(sizeLabel(freed) ?? '')
  })

  /**
   * Scenario 3, read twice. A removal that touches an adopted project names it; an object an
   * *unadopted* project can reach is excluded outright and the reason names the project to adopt.
   */
  test('a prune names the projects it reaches and the ones it refuses to touch', async ({
    page,
  }) => {
    await pane(page, 'containers-adopted', 'Volumes', 'volume')
    await page.getByTestId('object-prune-unused_volumes').click()
    // `shop` is adopted, so its volume may go — and the preview says whose work this reaches
    // before the confirmation is available, not after.
    await expect(page.getByTestId('operation-blast-radius')).toContainText('shop')
    await page.getByTestId('operation-dismiss').click()

    await openSection(page, 'Images')
    await page.getByTestId('object-prune-dangling_images').click()
    const excluded = page.getByTestId('operation-excluded')
    // Owned by nobody, reachable from `blog`, which is not adopted: left alone, with the project
    // named. Reachability decides, not ownership.
    await expect(excluded).toContainText('blog')
    await expect(page.getByTestId('operation-removals').locator('li')).toHaveCount(1)
    await expect(page.getByTestId('operation-removal-sha256:bbbb')).toBeVisible()
  })

  /**
   * Scenario 4. The confirmation is the object's own name, so it changes with what is being
   * removed; and the runtime is read again afterwards, so a success it did not earn is reported.
   */
  test('a removal is confirmed by name and verified against what actually happened', async ({
    page,
  }) => {
    await pane(page, 'containers-lying-runtime', 'Volumes', 'volume')
    await page.getByTestId('object-remove-shop_db_data').click()

    // The required string is the object's own name, so it changes with what is being removed and
    // cannot be muscle-memoried. Until it matches, the control is not merely refused on submit —
    // it cannot be pressed at all, which is a guard the operator cannot confirm past.
    const required = await typeConfirmation(page, 'remove')
    expect(required).toBe('shop_db_data')
    await expect(page.getByTestId('operation-issue')).toBeDisabled()

    await typeConfirmation(page)
    await expect(page.getByTestId('operation-issue')).toBeEnabled()
    await page.getByTestId('operation-issue').click()

    // The runtime called it a success. The reading afterwards still holds the volume, and that is
    // what is reported — a green tick over a removal that did not happen is the failure FR-023
    // exists to prevent.
    const discrepancies = page.getByTestId('operation-discrepancies')
    await expect(discrepancies).toBeVisible()
    await expect(discrepancies).toContainText('shop_db_data')
    await expect(discrepancies).toContainText('still present')
  })

  /** Scenario 5. The row says who holds it; if it is issued anyway, the daemon's own sentence. */
  test('a volume a container holds says so, and its refusal is reported verbatim', async ({
    page,
  }) => {
    await pane(page, 'containers-volume-in-use', 'Volumes', 'volume')
    await expect(page.getByTestId('object-references-in_use_data')).toHaveText('shop-web (running)')
    await expect(page.getByTestId('object-row-in_use_data')).toHaveAttribute('data-in-use', 'true')

    await page.getByTestId('object-remove-in_use_data').click()
    await typeConfirmation(page)
    await page.getByTestId('operation-issue').click()

    const outcome = page.getByTestId('operation-outcome')
    await expect(outcome).toHaveAttribute('data-overall', 'failed')
    // Verbatim. A rewritten error is the one the operator cannot search for.
    await expect(outcome).toContainText(volumeInUse('in_use_data', 'shop-web-id'))
  })

  /** Scenario 6. Cancelling is not a quiet issue: nothing was sent and nothing changed. */
  test('a cancelled removal issues nothing', async ({ page }) => {
    await pane(page, 'containers-adopted', 'Volumes', 'volume')
    await page.getByTestId('object-remove-shop_db_data').click()
    await expect(page.getByTestId('operation-preview')).toBeVisible()

    await page.getByTestId('operation-dismiss').click()
    await expect(page.getByTestId('operation-preview')).toHaveCount(0)
    expect(await sentTypes(page)).not.toContain('IssueContainerOperation')
    // And the object is still there, offered, exactly as it was.
    await expect(page.getByTestId('object-remove-shop_db_data')).toBeVisible()
  })

  /**
   * FR-027, the other half of exclusion: a selection in which every object is excluded has nothing
   * left in it, and is not offered as a removal of nothing. A typed confirmation over a no-op is
   * how an operator learns the guard is noise; the sentence they can act on names what to adopt.
   */
  test('a prune with nothing left to do is not offered, and names what to adopt', async ({
    page,
  }) => {
    await pane(page, 'containers', 'Networks', 'network')
    // Both networks belong to projects, and nothing is adopted in this scenario.
    expect(byName('shop_default').project).toBe('shop')
    await page.getByTestId('object-prune-unused_networks').click()

    // Refused against this request, so the answer is on the control that asked rather than a
    // sentence in the status bar belonging to nothing in particular.
    const refusal = page.getByTestId('operation-refusal')
    await expect(refusal).toHaveAttribute('data-kind', 'not_previewable')
    await expect(refusal).toHaveText(
      'Every object in that selection belongs to a project that has not been adopted: ' +
        'blog, shop. Adopt one to act on its objects.',
    )
    // And nothing is offered: no removal list, and no control to issue a removal of nothing.
    await expect(page.getByTestId('operation-removals')).toHaveCount(0)
    await expect(page.getByTestId('operation-issue')).toHaveCount(0)
  })
})
