/**
 * LIVE quickstart validation for 002-pod-logs-terminal (T034), Scenarios 1 and 2, against the
 * real backend through the devbridge. Wide viewport only (`--project chromium-wide`): the list and
 * the inspector sit side by side, which the single-session step needs.
 *
 * Throwaway pods (labelled `fathom.dev/e2e=true`, removed afterwards):
 * - `qs002-multi`: two busybox containers `a`/`b`, each printing its own name once a second;
 * - `qs002-second`: one busybox container, the second pod for the single-session step;
 * - `qs002-noshell`: `pause`, which has no shell binary (FR-013).
 * The sandbox's `crashloop` pod is the stopped-container case (FR-006).
 *
 * "No orphaned session" (FR-011, SC-003) is checked server-side: a `sleep 4242` started in the
 * terminal must be gone from the container once the session is closed.
 */
import { execFileSync } from 'node:child_process'

import type { Locator, Page } from '@playwright/test'

import { liveContext } from './env'
import {
  E2E_LABEL,
  expect,
  inspector,
  kubectl,
  LIVE_NAMESPACE,
  navigateTo,
  PODS,
  search,
  row,
  tapBridge,
  test,
  type BridgeTap,
  type KubeObject,
  type LiveApp,
} from './helpers'

const MULTI = 'qs002-multi'
const SECOND = 'qs002-second'
const NOSHELL = 'qs002-noshell'

const RESTRICTED = {
  runAsNonRoot: true,
  runAsUser: 65534,
  runAsGroup: 65534,
  seccompProfile: { type: 'RuntimeDefault' },
}
const CONTAINER_SEC = { allowPrivilegeEscalation: false, capabilities: { drop: ['ALL'] } }
const RES = { requests: { cpu: '5m', memory: '8Mi' }, limits: { cpu: '50m', memory: '32Mi' } }

function busybox(name: string, script: string) {
  return {
    name,
    image: 'busybox:1.37',
    command: ['sh', '-c', script],
    resources: RES,
    securityContext: CONTAINER_SEC,
  }
}
function pod(name: string, containers: unknown[]): KubeObject {
  return {
    apiVersion: 'v1',
    kind: 'Pod',
    metadata: { name, namespace: LIVE_NAMESPACE, labels: { [E2E_LABEL[0]]: E2E_LABEL[1] } },
    spec: { terminationGracePeriodSeconds: 1, securityContext: RESTRICTED, containers },
  }
}
const echoLoop = (word: string) => `while true; do echo "from-${word} $(date +%s)"; sleep 1; done`

/** `ps` inside a container (read-only exec, outside the UI; kubectl() forbids `exec`). */
function ps(name: string, container: string): string {
  return execFileSync(
    'kubectl',
    ['--context', liveContext(), '-n', LIVE_NAMESPACE, 'exec', name, '-c', container, '--', 'ps'],
    { encoding: 'utf8' },
  )
}
function podName(app: string): string {
  return kubectl(['get', 'pods', '-l', `app=${app}`, '-o', 'jsonpath={.items[0].metadata.name}'])
}

async function openPod(page: Page, name: string): Promise<Locator> {
  await navigateTo(page, 'Pods')
  await search(page, name, PODS)
  await row(page, `${LIVE_NAMESPACE}/${name}`, PODS).click()
  const insp = inspector(page)
  await expect(insp.getByRole('heading', { name })).toBeVisible()
  return insp
}

/** Terminal tab → (container) → 024 gate → attached xterm. */
async function openTerminal(page: Page, insp: Locator, container?: string): Promise<Locator> {
  await insp.getByRole('tab', { name: 'Terminal' }).click()
  if (container) {
    await insp.getByRole('combobox', { name: 'Container' }).click()
    await page.getByRole('option', { name: container, exact: true }).click()
  }
  await page.getByTestId('confirm-submit').click()
  return insp.getByTestId('pod-terminal-xterm')
}

async function run(page: Page, xterm: Locator, line: string): Promise<void> {
  await xterm.click()
  await page.keyboard.type(line)
  await page.keyboard.press('Enter')
}

const types = (tap: BridgeTap) => tap.sent.map((c) => c.type)

/** Reconnect after `tapBridge` so the new socket goes through the tap. */
async function reopen(app: LiveApp): Promise<void> {
  await app.open()
  await app.selectContext()
  await app.selectNamespace(LIVE_NAMESPACE)
}

test.describe('live 002 quickstart', () => {
  test.beforeAll(() => {
    // A failed test restarts the worker, re-running this hook while the previous afterAll's
    // pods may still be terminating: clear them first so `create` does not hit AlreadyExists.
    kubectl(['delete', 'pod', MULTI, SECOND, NOSHELL, '--ignore-not-found', '--wait=true'])
    kubectl(['create', '-f', '-'], {
      input: {
        apiVersion: 'v1',
        kind: 'List',
        items: [
          pod(MULTI, [busybox('a', echoLoop('a')), busybox('b', echoLoop('b'))]),
          pod(SECOND, [busybox('main', 'sleep infinity')]),
          pod(NOSHELL, [
            {
              name: 'main',
              image: 'registry.k8s.io/pause:3.10',
              resources: RES,
              securityContext: CONTAINER_SEC,
            },
          ]),
        ],
      },
    })
    kubectl([
      'wait',
      '--for=condition=Ready',
      `pod/${MULTI}`,
      `pod/${SECOND}`,
      `pod/${NOSHELL}`,
      '--timeout=120s',
    ])
  })
  test.afterAll(() => {
    kubectl(['delete', 'pod', MULTI, SECOND, NOSHELL, '--ignore-not-found', '--wait=false'])
  })

  test.beforeEach(async ({ app }) => {
    await app.open()
    await app.selectContext()
    await app.selectNamespace(LIVE_NAMESPACE)
  })

  test('S1.1-1.3 logs appear within 5s (SC-001), then live lines', async ({ page }) => {
    const name = podName('chatty-logger')
    const insp = await openPod(page, name)
    const scroll = insp.getByTestId('pod-logs-scroll')
    const t0 = Date.now()
    await insp.getByRole('tab', { name: 'Logs' }).click()
    await expect(scroll.locator('pre')).not.toBeEmpty({ timeout: 5_000 })
    const firstLines = Date.now() - t0
    test.info().annotations.push({ type: 'SC-001 ms', description: String(firstLines) })
    expect(firstLines).toBeLessThan(5_000)
    const before = (await scroll.textContent()) ?? ''
    await expect
      .poll(async () => (await scroll.textContent()) ?? '', { timeout: 20_000 })
      .not.toBe(before)
  })

  test('S1.4-1.6 container picker, auto-scroll + jump to latest, stream stops on leave', async ({
    app,
    page,
  }) => {
    const tap = await tapBridge(page)
    await reopen(app)
    const insp = await openPod(page, MULTI)
    await insp.getByRole('tab', { name: 'Logs' }).click()
    const scroll = insp.getByTestId('pod-logs-scroll')
    await expect(scroll).toContainText('from-a')
    await expect(scroll).not.toContainText('from-b')

    // Switching containers restarts the stream with the other container's output.
    await insp.getByRole('combobox', { name: 'Container' }).click()
    await page.getByRole('option', { name: 'b', exact: true }).click()
    await expect(scroll).toContainText('from-b')
    await expect(scroll).not.toContainText('from-a')
    const starts = tap.sent.filter((c) => c.type === 'StartPodLogs')
    expect(starts.at(-1)).toMatchObject({ data: { name: MULTI, container: 'b' } })

    // Wait for enough lines to scroll, scroll up: follow stops and "Jump to latest" appears.
    await expect
      .poll(() => scroll.evaluate((el) => el.scrollHeight > el.clientHeight + 100), {
        timeout: 60_000,
      })
      .toBe(true)
    await scroll.evaluate((el) => {
      el.scrollTop = 0
      el.dispatchEvent(new Event('scroll'))
    })
    const jump = insp.getByRole('button', { name: 'Jump to latest' })
    await expect(jump).toBeVisible()
    await page.waitForTimeout(2_500) // new lines arrive; the view must stay put
    expect(await scroll.evaluate((el) => el.scrollTop)).toBe(0)
    await jump.click()
    await expect(jump).toBeHidden()
    await page.waitForTimeout(2_500)
    expect(
      await scroll.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight),
    ).toBeLessThan(40)

    // Leaving the tab stops the stream (FR-005).
    const sentBefore = tap.sent.length
    await insp.getByRole('tab', { name: 'Details' }).click()
    await expect.poll(() => types(tap).slice(sentBefore)).toContain('StopPodLogs')
  })

  test('S1.7 a crashed container shows a distinct "stream ended" indicator', async ({ page }) => {
    const insp = await openPod(page, podName('crashloop'))
    await insp.getByRole('tab', { name: 'Logs' }).click()
    await expect(insp.locator('[data-pane-state="stream-ended"]')).toBeVisible({ timeout: 30_000 })
  })

  test('S2.1-2.5 terminal: picker first, ls output within 10s, resize reaches the pty', async ({
    app,
    page,
  }) => {
    const tap = await tapBridge(page)
    await reopen(app)
    const insp = await openPod(page, MULTI)
    await insp.getByRole('tab', { name: 'Terminal' }).click()
    // FR-008: the picker comes before any session.
    await expect(insp.getByRole('combobox', { name: 'Container' })).toBeVisible()
    expect(types(tap)).not.toContain('StartPodExec')
    await insp.getByRole('combobox', { name: 'Container' }).click()
    await page.getByRole('option', { name: 'b', exact: true }).click()
    const t0 = Date.now()
    await page.getByTestId('confirm-submit').click()
    const xterm = insp.getByTestId('pod-terminal-xterm')
    await expect(xterm).toBeVisible()
    expect(tap.sent.find((c) => c.type === 'StartPodExec')).toMatchObject({
      data: { name: MULTI, container: 'b' },
    })

    await run(page, xterm, 'ls /')
    await expect(xterm).toContainText('proc', { timeout: 10_000 })
    await expect(xterm).toContainText('etc')
    const sc002 = Date.now() - t0
    test.info().annotations.push({ type: 'SC-002 ms', description: String(sc002) })
    expect(sc002).toBeLessThan(10_000)

    // FR-010: the remote pty has the local terminal's size (busybox `ttysize`; `stty size`
    // fails with ENOENT on this kind node), and a pane resize reaches it too.
    const lastResize = (): string | undefined => {
      const r = tap.sent.filter((c) => c.type === 'ResizePodExec').at(-1)
      return r?.type === 'ResizePodExec' ? `${r.data.cols} ${r.data.rows}` : undefined
    }
    await expect.poll(lastResize).toBeDefined()
    const sent1 = lastResize()
    await run(page, xterm, 'echo SZ1 $(ttysize) END')
    await expect(xterm).toContainText(/SZ1 \d+ \d+ END/)
    const size1 = /SZ1 (\d+ \d+) END/.exec((await xterm.textContent()) ?? '')?.[1]
    expect(size1).toBe(sent1)
    // Stay inside the wide class (1920-2559) so the pane grows instead of the layout reflowing.
    await page.setViewportSize({ width: 2400, height: 1300 })
    await expect.poll(lastResize, { timeout: 15_000 }).not.toBe(sent1)
    const sent2 = lastResize()
    await run(page, xterm, 'echo SZ2 $(ttysize) END')
    await expect(xterm).toContainText(/SZ2 \d+ \d+ END/)
    const size2 = /SZ2 (\d+ \d+) END/.exec((await xterm.textContent()) ?? '')?.[1]
    test
      .info()
      .annotations.push({ type: 'pty size', description: `${String(size1)} -> ${String(size2)}` })
    expect(size2).toBe(sent2)
    expect(size2).not.toBe(size1)
  })

  test('S2.6-2.7 closing ends the session server-side; a second pod ends the first', async ({
    app,
    page,
  }) => {
    const tap = await tapBridge(page)
    await reopen(app)

    // 2.6: switch away from the tab → the shell (and its child) is gone in the container.
    let insp = await openPod(page, SECOND)
    let xterm = await openTerminal(page, insp)
    await run(page, xterm, 'sleep 4242')
    await expect.poll(() => ps(SECOND, 'main'), { timeout: 15_000 }).toContain('sleep 4242')
    await insp.getByRole('tab', { name: 'Details' }).click()
    await expect.poll(() => types(tap)).toContain('StopPodExec')
    await expect.poll(() => ps(SECOND, 'main'), { timeout: 30_000 }).not.toContain('sleep 4242')

    // 2.7: a session on pod A, then Terminal on pod B: A is stopped before B starts.
    insp = await openPod(page, SECOND)
    xterm = await openTerminal(page, insp)
    await run(page, xterm, 'sleep 4243')
    await expect.poll(() => ps(SECOND, 'main'), { timeout: 15_000 }).toContain('sleep 4243')
    const mark = tap.sent.length
    insp = await openPod(page, MULTI)
    xterm = await openTerminal(page, insp, 'a')
    await run(page, xterm, 'echo second-session-ok')
    await expect(xterm).toContainText('second-session-ok')
    const after = types(tap).slice(mark)
    expect(after.indexOf('StopPodExec')).toBeGreaterThanOrEqual(0)
    expect(after.indexOf('StartPodExec')).toBeGreaterThan(after.indexOf('StopPodExec'))
    await expect.poll(() => ps(SECOND, 'main'), { timeout: 30_000 }).not.toContain('sleep 4243')
  })

  test('S2.8 a container with no shell shows a clear error, not a hang', async ({ page }) => {
    const insp = await openPod(page, NOSHELL)
    await openTerminal(page, insp)
    const err = insp.locator(
      '[data-pane-state="terminal-error"], [data-pane-state="terminal-ended"]',
    )
    await expect(err).toBeVisible({ timeout: 20_000 })
    const text = (await err.textContent()) ?? ''
    test.info().annotations.push({ type: 'no-shell message', description: text })
    expect(text.trim().length).toBeGreaterThan(10)
  })
})
