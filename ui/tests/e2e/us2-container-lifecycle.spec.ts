/**
 * 048-docker-management US2: act on what is running, on a daemon that is not ours alone.
 *
 * The eight acceptance scenarios of the story, and between them one claim: nothing here acts
 * without the operator having seen the exact command first, and nothing acts at all on a project
 * they have not adopted. The daemon under this scene is shared with unrelated projects, so
 * "read-only until adopted" is not a nicety — it is what stops a mis-click reaching someone
 * else's work.
 *
 * Two of these are worth reading twice. A project operation is one argv over several containers,
 * so its outcome has to be several answers and not one: a run where one member refused must not
 * report overall success (FR-022). And a container that vanished between the preview and the
 * confirmation must be reported in the runtime's own words — "no such container" — rather than
 * quietly dropped from the report, because the operator's next move depends on knowing which it
 * was (FR-023).
 */
import { expect, test, type Page } from '@playwright/test'

import { ALL_CONTAINERS, noSuchContainer } from '../fixtures/containers'
import { open, sentMutations, sentTypes } from './helpers'

/** The group and its first section share the label "Containers", so the click is scoped by row. */
async function openContainers(page: Page, label = 'Containers'): Promise<void> {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav).toBeVisible()
  const toggle = nav.locator('button[aria-expanded]', { hasText: 'Containers' }).first()
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
  await nav.locator('li button', { hasText: label }).first().click()
}

/** Wait for the one-shot presence probe to have been answered. */
async function probed(page: Page): Promise<void> {
  await expect.poll(async () => await sentTypes(page)).toContain('RefreshContainerRuntimePresence')
}

/** The scene, with the inventory on screen. */
async function scene(page: Page, scenario: Parameters<typeof open>[1]): Promise<void> {
  await open(page, scenario)
  await probed(page)
  await openContainers(page)
  await expect(page.getByTestId('container-group-list')).toBeVisible()
}

/** The daemon-assigned id of a fixture container, which is what every argv is written in terms of. */
function idOf(name: string): string {
  const found = ALL_CONTAINERS.find((c) => c.name === name)
  if (!found) throw new Error(`no fixture container named ${name}`)
  return found.id
}

/** Confirm and issue a previewed operation, typing the confirmation only when one is asked for. */
async function issue(page: Page): Promise<void> {
  const field = page.getByTestId('operation-confirmation')
  if (await field.isVisible()) {
    // The required string is only ever on screen, in the label beside the field — reading it from
    // there is the operator's own route, and a test that hardcoded it would pass a UI that asked
    // for something else.
    const required = await page.locator('label[for="operation-confirmation"] .font-mono').textContent()
    await field.fill((required ?? '').trim())
  }
  await page.getByTestId('operation-issue').click()
}

test.describe('US2 container lifecycle', () => {
  test('an unadopted project is readable, offers no verb, and says why', async ({ page }) => {
    await scene(page, 'containers')
    await page.getByTestId('containers-group-blog').click()

    // Readable: the members and their states are on screen whether or not Fathom may touch them.
    await expect(page.getByTestId('container-row-blog-web')).toBeVisible()
    await expect(page.getByTestId('container-row-blog-worker')).toBeVisible()

    const control = page.getByTestId('adoption-control-blog')
    await expect(control).toHaveAttribute('data-adoption', 'not_adopted')
    await expect(page.getByTestId('adoption-explanation-blog')).toContainText('Read-only')
    // Stating the rule is only half of it: the route out of read-only is offered in the same
    // place, so the operator is not left to hunt for it (FR-017).
    await expect(page.getByTestId('adoption-toggle-blog')).toHaveText(/Adopt blog/)

    // And not one mutating verb anywhere on the panel.
    await expect(page.getByTestId('project-stop-blog')).toHaveCount(0)
    await expect(page.getByTestId('project-restart-blog')).toHaveCount(0)
    await expect(page.getByTestId('project-down-blog')).toHaveCount(0)
  })

  test('adopting a project opens the verbs for every container in it', async ({ page }) => {
    await scene(page, 'containers')
    await page.getByTestId('containers-group-blog').click()
    await page.getByTestId('adoption-toggle-blog').click()

    await expect(page.getByTestId('adoption-control-blog')).toHaveAttribute(
      'data-adoption',
      'adopted',
    )
    await expect(page.getByTestId('project-restart-blog')).toBeVisible()
    // Every member, not just the one that happened to be selected: the grant is over the project.
    for (const name of ['blog-web', 'blog-worker']) {
      await page.getByTestId(`container-row-${name}`).click()
      await expect(page.getByTestId(`container-actions-${name}`)).toBeVisible()
    }
    // The grant is recorded by the backend, not inferred by the UI from a click.
    expect(await sentTypes(page)).toContain('AdoptContainerProject')
  })

  test('a grant recorded earlier is already in force when the scene opens', async ({ page }) => {
    // This is adoption surviving a restart, as a scene can observe it: the fake's state is
    // per-page, so a reload cannot test the daemon's own memory. What it can test is the half
    // that is this scene's job — a grant that arrives with the inventory governs it immediately,
    // without the operator re-adopting anything (FR-014).
    await scene(page, 'containers-adopted')
    await page.getByTestId('containers-group-shop').click()
    await expect(page.getByTestId('adoption-control-shop')).toHaveAttribute(
      'data-adoption',
      'adopted',
    )
    await expect(page.getByTestId('project-restart-shop')).toBeVisible()
    expect(await sentTypes(page)).not.toContain('AdoptContainerProject')
  })

  test('releasing a project returns it to read-only', async ({ page }) => {
    await scene(page, 'containers-adopted')
    await page.getByTestId('containers-group-shop').click()
    await expect(page.getByTestId('adoption-toggle-shop')).toHaveText(/Release shop/)
    await page.getByTestId('adoption-toggle-shop').click()

    await expect(page.getByTestId('adoption-control-shop')).toHaveAttribute(
      'data-adoption',
      'not_adopted',
    )
    await expect(page.getByTestId('project-restart-shop')).toHaveCount(0)
    await page.getByTestId('container-row-shop-web').click()
    await expect(page.getByTestId('container-actions-shop-web')).toHaveCount(0)
  })

  test('a container stop shows its exact command and waits to be confirmed', async ({ page }) => {
    await scene(page, 'containers-adopted')
    await page.getByTestId('container-row-shop-web').click()
    await page.getByTestId('container-stop-shop-web').click()

    const preview = page.getByTestId('operation-preview')
    await expect(preview).toHaveAttribute('data-phase', 'previewed')
    // The argv, whole, before anything runs — and scoped to the one container that was chosen,
    // so a sibling is not in it.
    await expect(page.getByTestId('operation-command')).toHaveText(
      `docker stop ${idOf('shop-web')}`,
    )
    await expect(page.getByTestId('operation-command')).not.toContainText(idOf('shop-db'))
    // Previewing is not issuing. Nothing has been asked of the runtime yet.
    expect(await sentTypes(page)).not.toContain('IssueContainerOperation')
  })

  test('issuing carries only the preview, and the row waits for the runtime', async ({ page }) => {
    await scene(page, 'containers-adopted')
    const before = await page.getByTestId('container-state-shop-web').textContent()
    await page.getByTestId('container-row-shop-web').click()
    await page.getByTestId('container-stop-shop-web').click()
    await issue(page)

    const outcome = page.getByTestId('operation-outcome')
    await expect(outcome).toHaveAttribute('data-overall', 'succeeded')
    await expect(page.getByTestId(`operation-outcome-${idOf('shop-web')}`)).toHaveAttribute(
      'data-status',
      'succeeded',
    )
    // The UI never re-derives the argv: issuing carries the preview's id and nothing that could
    // disagree with what was on screen (FR-020).
    const issued = (await sentMutations(page))
      .filter((m) => (m as { type: string }).type === 'IssueContainerOperation')
      .map((m) => (m as { data: Record<string, unknown> }).data)
    expect(issued).toHaveLength(1)
    expect(Object.keys(issued[0] ?? {}).sort()).toEqual([
      'preview_id',
      'request_id',
      'typed_confirmation',
    ])
    // And the row still says what the last reading of the daemon said. A stop that succeeded is
    // not a reading; the next inventory is (FR-021).
    expect(await page.getByTestId('container-state-shop-web').textContent()).toBe(before)
  })

  test('a failure is reported in the runtime’s own words', async ({ page }) => {
    await scene(page, 'containers-partial-failure')
    await page.getByTestId('container-row-shop-db').click()
    await page.getByTestId('container-stop-shop-db').click()
    await issue(page)

    await expect(page.getByTestId('operation-outcome')).toHaveAttribute('data-overall', 'failed')
    const line = page.getByTestId(`operation-outcome-${idOf('shop-db')}`)
    await expect(line).toHaveAttribute('data-status', 'failed')
    await expect(line).toContainText('permission denied')
  })

  test('a project restart names every member and answers for each of them', async ({ page }) => {
    await scene(page, 'containers-adopted')
    await page.getByTestId('containers-group-shop').click()
    await page.getByTestId('project-restart-shop').click()

    // One argv for the whole project, with both members in it — which is exactly why the outcome
    // cannot be a single verdict.
    const command = page.getByTestId('operation-command')
    await expect(command).toContainText(idOf('shop-db'))
    await expect(command).toContainText(idOf('shop-web'))
    await issue(page)

    for (const name of ['shop-web', 'shop-db']) {
      await expect(page.getByTestId(`operation-outcome-${idOf(name)}`)).toHaveAttribute(
        'data-status',
        'succeeded',
      )
    }
  })

  test('one member failing does not let the run report success', async ({ page }) => {
    await scene(page, 'containers-partial-failure')
    await page.getByTestId('containers-group-shop').click()
    await page.getByTestId('project-stop-shop').click()
    await issue(page)

    const outcome = page.getByTestId('operation-outcome')
    // Not "not succeeded by implication": the overall verdict is the failure itself, because an
    // operator who reads only the summary must not come away believing the project stopped.
    await expect(outcome).toHaveAttribute('data-overall', 'failed')
    await expect(page.getByTestId(`operation-outcome-${idOf('shop-web')}`)).toHaveAttribute(
      'data-status',
      'succeeded',
    )
    await expect(page.getByTestId(`operation-outcome-${idOf('shop-db')}`)).toHaveAttribute(
      'data-status',
      'failed',
    )
  })

  test('a container gone since the preview is reported, not dropped', async ({ page }) => {
    await scene(page, 'containers-vanished-member')
    await page.getByTestId('container-row-shop-db').click()
    await page.getByTestId('container-stop-shop-db').click()
    await issue(page)

    const line = page.getByTestId(`operation-outcome-${idOf('shop-db')}`)
    await expect(line).toHaveAttribute('data-status', 'failed')
    // Verbatim. A paraphrase would be Fathom's guess at what the daemon meant.
    await expect(line).toContainText(noSuchContainer('shop-db'))
    // The preview named one container, so one container is all the report may be about: no
    // sibling was touched on the way.
    await expect(page.getByTestId(`operation-outcome-${idOf('shop-web')}`)).toHaveCount(0)
    // And the difference between what was expected and what was there is stated as such.
    await expect(page.getByTestId('operation-discrepancies')).toContainText('shop-db')
  })
})
