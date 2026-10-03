/**
 * 048-docker-management US5: a container's own words, and a shell in one that was adopted.
 *
 * The five acceptance scenarios, and between them two claims. Reading is free and changing is not:
 * a container's logs are followable whether or not its project was adopted (FR-035), while a
 * session is offered only where a mutation would be (FR-013) — and where it is refused, the
 * refusal is the backend's own sentence and not an empty terminal (FR-004).
 *
 * The other claim is about what is left running. A follow is a process on this machine for as long
 * as its pane is on screen, so every way of leaving that pane — closing it, selecting another
 * container, leaving the section — has to end it. Three of these tests are that one sentence asked
 * three ways, because "no process left behind" is the requirement an operator cannot check for
 * themselves.
 */
import { expect, test, type Page } from '@playwright/test'

import { ALL_CONTAINERS } from '../fixtures/containers'
import { open, sent, sentTypes } from './helpers'

/** The group and its first section share the label "Containers", so the click is scoped by row. */
async function openContainers(page: Page): Promise<void> {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav).toBeVisible()
  const toggle = nav.locator('button[aria-expanded]', { hasText: 'Containers' }).first()
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
  await nav.locator('li button', { hasText: 'Containers' }).first().click()
}

/** The scene, with the inventory on screen. */
async function scene(page: Page, scenario: Parameters<typeof open>[1]): Promise<void> {
  await open(page, scenario)
  await expect.poll(async () => await sentTypes(page)).toContain('RefreshContainerRuntimePresence')
  await openContainers(page)
  await expect(page.getByTestId('container-group-list')).toBeVisible()
}

function idOf(name: string): string {
  const found = ALL_CONTAINERS.find((c) => c.name === name)
  if (!found) throw new Error(`no fixture container named ${name}`)
  return found.id
}

/** Select one container by name, which is what puts its panes on screen. */
async function select(page: Page, group: string, name: string): Promise<string> {
  await page.getByTestId(`containers-group-${group}`).click()
  await page.getByTestId(`container-row-${name}`).click()
  const id = idOf(name)
  await expect(page.getByTestId(`container-streams-${id}`)).toBeVisible()
  return id
}

/** The stream ids a given command was sent for, in order. */
async function streamIdsFor(page: Page, type: string): Promise<number[]> {
  const commands = await sent(page)
  return commands
    .filter((c) => c.type === type)
    .map((c) => ('data' in c ? (c.data as { stream_id: number }).stream_id : -1))
}

test.describe('US5 container logs and session', () => {
  test('a follow arrives live, and the lines it dropped are stated', async ({ page }) => {
    // `containers-chatty` is the same daemon with a follow that has already dropped lines: a bound
    // nobody is told about is indistinguishable from a bug (FR-034).
    await scene(page, 'containers-chatty')
    const id = await select(page, 'shop', 'shop-web')

    await page.getByTestId(`container-logs-open-${id}`).click()
    const scroll = page.getByTestId(`container-logs-scroll-${id}`)
    await expect(scroll).toBeVisible()
    await expect(scroll).toContainText('listening on :80')
    await expect(scroll).toHaveAttribute('data-stream-status', 'streaming')
    await expect(scroll).toContainText('truncated')

    // The follow is one command naming the stream the pane minted, and it asks to follow.
    const started = (await sent(page)).find((c) => c.type === 'StartContainerLogStream')
    expect(started?.data).toMatchObject({ container_id: id, follow: true })
  })

  test('a stopped container keeps its output, says it is not running, and leaves no stream open', async ({
    page,
  }) => {
    // Unadopted on purpose: reading a container's output changes nothing about it, so the grant
    // that gates a shell must not gate this (FR-035).
    await scene(page, 'containers')
    const id = await select(page, 'blog', 'blog-worker')

    await page.getByTestId(`container-logs-open-${id}`).click()
    const scroll = page.getByTestId(`container-logs-scroll-${id}`)
    await expect(scroll).toContainText('listening on :80')
    await expect(page.getByTestId(`container-logs-not-running-${id}`)).toContainText('not running')
    // The runtime's own status line, so the operator reads how long ago rather than "stopped".
    await expect(page.getByTestId(`container-logs-not-running-${id}`)).toContainText('Exited (137)')

    // Nothing is left following a container that cannot produce another line.
    await expect(scroll).toHaveAttribute('data-stream-status', 'closed')
    await expect(
      page
        .getByTestId('pane-state-stream-ended')
        .or(page.locator('[data-pane-state="stream-ended"]')),
    ).toBeVisible()
  })

  test('a shell in an adopted container takes input and ends when the pane closes', async ({
    page,
  }) => {
    await scene(page, 'containers-adopted')
    const id = await select(page, 'shop', 'shop-web')

    await page.getByTestId(`container-shell-open-${id}`).click()
    const term = page.getByTestId(`container-shell-${id}`)
    await expect(term).toBeVisible()
    // The prompt the fake's shell writes. Until it arrives the surface is still attaching, so this
    // is also the assertion that the session came up.
    await expect(term).toContainText('/ #')

    await term.click()
    await page.keyboard.type('id')
    await page.keyboard.press('Enter')
    // Input is plain text on the wire for a container session — unlike a pod exec, which is
    // base64. The echo proves the round trip rather than the keystroke merely being sent.
    await expect
      .poll(async () => (await sentTypes(page)).join())
      .toContain('SendContainerSessionInput')
    await expect(term).toContainText('id')

    // FR-036: closing the view ends the session, for the same stream it opened.
    const [opened] = await streamIdsFor(page, 'StartContainerSession')
    await page.getByTestId(`container-stream-close-${id}`).click()
    await expect(term).toBeHidden()
    await expect
      .poll(async () => await streamIdsFor(page, 'StopContainerSession'))
      .toEqual([opened])
  })

  test('a container with no usable shell reports the runtime verbatim, not an empty terminal', async ({
    page,
  }) => {
    await scene(page, 'containers-no-shell')
    const id = await select(page, 'shop', 'shop-web')

    await page.getByTestId(`container-shell-open-${id}`).click()
    // Verbatim down to the "unknown" the runtime ends on: a reworded failure is a failure the
    // operator cannot search for (FR-004).
    await expect(page.getByTestId(`container-streams-${id}`)).toContainText(
      'stat /bin/sh: no such file or directory: unknown',
    )
  })

  test('an unadopted container offers logs and no shell, and says which', async ({ page }) => {
    await scene(page, 'containers')
    const id = await select(page, 'blog', 'blog-web')

    await expect(page.getByTestId(`container-logs-open-${id}`)).toBeVisible()
    await expect(page.getByTestId(`container-shell-open-${id}`)).toHaveCount(0)
    await expect(page.getByTestId(`container-shell-denied-${id}`)).toContainText('Adopt')
  })

  test('leaving the section leaves nothing streaming', async ({ page }) => {
    await scene(page, 'containers-adopted')
    const id = await select(page, 'shop', 'shop-web')

    await page.getByTestId(`container-logs-open-${id}`).click()
    await expect(page.getByTestId(`container-logs-scroll-${id}`)).toBeVisible()
    const [followed] = await streamIdsFor(page, 'StartContainerLogStream')

    // Navigating away unmounts the pane, which is what ends the follow: nothing has to remember
    // to stop it (FR-045's sibling for logs).
    const nav = page.getByRole('navigation', { name: 'Sections' })
    if (!(await nav.isVisible())) {
      await page.getByRole('button', { name: 'Open sections' }).click()
    }
    await nav.locator('li button', { hasText: 'Pods' }).first().click()
    await expect(page.getByTestId(`container-logs-scroll-${id}`)).toBeHidden()

    await expect
      .poll(async () => await streamIdsFor(page, 'StopContainerLogStream'))
      .toEqual([followed])
  })

  test('selecting another container ends the first one’s follow', async ({ page }) => {
    await scene(page, 'containers-adopted')
    const first = await select(page, 'shop', 'shop-web')

    await page.getByTestId(`container-logs-open-${first}`).click()
    await expect(page.getByTestId(`container-logs-scroll-${first}`)).toBeVisible()
    const [followed] = await streamIdsFor(page, 'StartContainerLogStream')

    // A pane left open under another container's name would be showing one container's output as
    // another's, with both streams running.
    const second = await select(page, 'shop', 'shop-db')
    await expect(page.getByTestId(`container-logs-scroll-${first}`)).toBeHidden()
    await expect(page.getByTestId(`container-logs-scroll-${second}`)).toHaveCount(0)
    await expect
      .poll(async () => await streamIdsFor(page, 'StopContainerLogStream'))
      .toEqual([followed])
  })
})
