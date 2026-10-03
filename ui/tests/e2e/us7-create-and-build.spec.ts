/**
 * 048-docker-management US7: creating things, and watching a build.
 *
 * Six acceptance scenarios over one claim: every operation Fathom offers can be described in
 * advance. The forms collect fields, the fields become one argv element each, and the operator
 * approves that argv before anything runs — which is why there is no box to type a command into
 * (FR-019) and why this spec asserts the absence of one as plainly as it asserts the presence of
 * everything else.
 *
 * The build is the exception that proves the rule. It is the one operation whose result arrives
 * over time rather than at the end, so its output is streamed and the failure is the builder's own
 * three lines, shown verbatim and then repeated verbatim in the outcome (FR-021). A build that
 * fails only says why in the middle of its output; a run that reported "failed" would have thrown
 * away the only part worth reading.
 *
 * And the limit is stated, not merely enforced: a case no form covers belongs in a terminal
 * (FR-034), and a project whose definition file Fathom cannot read loses its project-level verbs
 * with the file left exactly as it was (FR-033, Principle VI).
 */
import { expect, test, type Page } from '@playwright/test'

import { BUILD_FAILED_SOLVE } from '../fixtures/containers'
import { navigateTo, open, sent, sentTypes } from './helpers'

/**
 * The Containers group holds a section of its own name, so the click is scoped to the group's
 * own section list — which is what `navigateTo` does, and it also waits for the phone drawer to
 * finish closing before the next navigation touches it.
 */
async function openContainers(page: Page, label = 'Containers'): Promise<void> {
  await navigateTo(page, label, 'Containers')
}

/** One of the sibling object sections, by its own label. */
async function openSection(page: Page, label: string): Promise<void> {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  const toggle = nav.locator('button[aria-expanded]', { hasText: 'Containers' }).first()
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
  await nav.locator('li button', { hasText: label }).first().click()
}

async function scene(page: Page, scenario: Parameters<typeof open>[1]): Promise<void> {
  await open(page, scenario)
  await expect.poll(async () => await sentTypes(page)).toContain('RefreshContainerRuntimePresence')
  await openContainers(page)
  await expect(page.getByTestId('container-group-list')).toBeVisible()
}

/** The request of the last `PreviewContainerOperation` sent. */
async function lastPreviewRequest(page: Page): Promise<Record<string, unknown>> {
  const commands = await sent(page)
  const previews = commands.filter((c) => c.type === 'PreviewContainerOperation')
  const last = previews.at(-1)
  if (!last || !('data' in last)) throw new Error('no PreviewContainerOperation was sent')
  return (last.data as { request: Record<string, unknown> }).request
}

test.describe('US7 creation and build', () => {
  test('a container is created from fields, and every field is its own argument', async ({
    page,
  }) => {
    await scene(page, 'containers-adopted')
    await page.getByTestId('containers-create').click()
    const form = page.getByTestId('create-container-form')
    await expect(form).toBeVisible()

    await page.getByTestId('create-container-image').fill('busybox:latest')
    await page.getByTestId('create-container-name').fill('scratch-box')
    await page.getByTestId('create-container-port-add').click()
    await page.getByTestId('create-container-port-key-0').fill('8080')
    await page.getByTestId('create-container-port-value-0').fill('80')
    await page.getByTestId('create-container-env-add').click()
    await page.getByTestId('create-container-env-key-0').fill('GREETING')
    // A value a shell would have reinterpreted. It stays one value because it was collected as
    // one field and is sent as one field — which is the whole argument for forms over a command
    // line (FR-019).
    await page.getByTestId('create-container-env-value-0').fill('hello; rm -rf /')

    await page.getByTestId('create-container-preview').click()

    const request = await lastPreviewRequest(page)
    expect(request).toMatchObject({
      kind: 'create_container',
      spec: {
        image: 'busybox:latest',
        name: 'scratch-box',
        ports: [{ container_port: 80, host_port: 8080, protocol: 'tcp' }],
        env: [['GREETING', 'hello; rm -rf /']],
        detach: true,
      },
    })

    // The form ends at a preview, never at a run: the argv is still approved before anything
    // happens (FR-018).
    await expect(page.getByTestId('operation-preview')).toBeVisible()
    await expect(page.getByTestId('operation-command')).toContainText('busybox:latest')
    expect(await sentTypes(page)).not.toContain('IssueContainerOperation')
  })

  test('there is nowhere to type a command, and the form says why', async ({ page }) => {
    await scene(page, 'containers-adopted')
    await page.getByTestId('containers-create').click()
    const form = page.getByTestId('create-container-form')

    // FR-019 as the operator meets it: no free-text command field exists. The protocol guards the
    // same rule on its own side; this is the half an operator can see.
    for (const label of [/extra argument/i, /raw command/i, /command line/i, /arguments/i]) {
      await expect(form.getByLabel(label)).toHaveCount(0)
    }
    // A command is collected as words, each its own box — not as a line of shell.
    await page.getByTestId('create-container-word-add').click()
    await expect(page.getByTestId('create-container-word-0')).toBeVisible()

    // FR-034: the limit is stated rather than merely enforced. The place for a case no form covers
    // is a terminal, and saying so is the difference between a boundary and a missing feature.
    await expect(page.getByTestId('create-container-terminal-note')).toContainText('terminal')
  })

  test('a build streams the builder’s own words and repeats its failure verbatim', async ({
    page,
  }) => {
    await scene(page, 'containers-failing-build')
    await openSection(page, 'Images')
    await page.getByTestId('object-create-image').click()

    await page.getByTestId('build-image-tag').fill('my-app:dev')
    await page.getByTestId('build-image-context').fill('/srv/my-app')
    await page.getByTestId('build-image-preview').click()

    await expect(page.getByTestId('operation-command')).toContainText('my-app:dev')
    await page.getByTestId('operation-issue').click()

    // The output is on screen as the builder wrote it, layer lines and all.
    const output = page.getByTestId('operation-build-output')
    await expect(output).toBeVisible()
    await expect(output).toContainText('#4 [2/3] RUN npm ci')
    // And the reason, verbatim down to the exit code — a reworded build error is one nobody can
    // search for (FR-004).
    await expect(output).toContainText(BUILD_FAILED_SOLVE)

    // The outcome repeats the same sentence rather than summarising it, so the operator reads the
    // failure twice and both readings are the builder's.
    await expect(page.getByTestId('operation-preview')).toHaveAttribute('data-phase', 'finished')
    await expect(page.getByTestId('operation-preview')).toContainText(BUILD_FAILED_SOLVE)
  })

  test('a network and a volume are created from their own fields', async ({ page }) => {
    await scene(page, 'containers-adopted')

    await openSection(page, 'Networks')
    await page.getByTestId('object-create-network').click()
    await page.getByTestId('create-network-name').fill('shop-internal')
    await page.getByTestId('create-network-subnet').fill('172.30.0.0/16')
    await page.getByTestId('create-network-internal').click()
    await page.getByTestId('create-network-preview').click()
    expect(await lastPreviewRequest(page)).toMatchObject({
      kind: 'create_network',
      spec: { name: 'shop-internal', subnet: '172.30.0.0/16', internal: true },
    })
    await page.getByTestId('operation-dismiss').click()

    await openSection(page, 'Volumes')
    await page.getByTestId('object-create-volume').click()
    await page.getByTestId('create-volume-name').fill('shop-cache')
    await page.getByTestId('create-volume-preview').click()
    expect(await lastPreviewRequest(page)).toMatchObject({
      kind: 'create_volume',
      spec: { name: 'shop-cache' },
    })
  })

  test('compose down is destructive and asks for the project’s own name', async ({ page }) => {
    await scene(page, 'containers-adopted')
    await page.getByTestId('containers-group-shop').click()
    await page.getByTestId('project-down-shop').click()

    const preview = page.getByTestId('operation-preview')
    await expect(preview).toBeVisible()
    await expect(page.getByTestId('operation-command')).toContainText('compose')
    await expect(page.getByTestId('operation-command')).toContainText('down')
    // Every container it takes with it, named. A `down` leaves volumes alone, so none are listed:
    // warning about a loss that is not going to happen is its own kind of wrong.
    await expect(page.getByTestId('operation-blast-radius')).toContainText('shop')
    await expect(preview).toContainText('shop-db')

    // The project's own name, not a constant: a word that is the same every time is a reflex, and
    // a reflex confirms the wrong project as readily as the right one.
    const confirmation = page.getByTestId('operation-confirmation')
    await expect(confirmation).toBeVisible()
    await expect(page.getByTestId('operation-issue')).toBeDisabled()
    await confirmation.fill('shop')
    await expect(page.getByTestId('operation-issue')).toBeEnabled()
  })

  test('a project with no readable definition file loses its project verbs, and keeps its file', async ({
    page,
  }) => {
    await scene(page, 'containers-missing-definition')
    await page.getByTestId('containers-group-shop').click()

    // FR-033: the verbs are gone and the reason is on screen, including the sentence that says
    // Fathom did not write over the operator's file (Principle VI).
    await expect(page.getByTestId('project-up-shop')).toHaveCount(0)
    await expect(page.getByTestId('project-down-shop')).toHaveCount(0)
    const note = page.getByTestId('project-no-definition-shop')
    await expect(note).toContainText('no longer on disk')
    await expect(note).toContainText('Fathom has not changed the file')

    // Nothing was sent that would have touched it. The individual containers are still actionable,
    // which is the point of saying which operations went away rather than greying out the project.
    expect(await sentTypes(page)).not.toContain('IssueContainerOperation')
    await expect(page.getByTestId('project-stop-shop')).toBeVisible()
  })
})
