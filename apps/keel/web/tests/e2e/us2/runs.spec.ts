/**
 * US2 (spec 055 T064): the run list, a run's page with its live log, the per-host results with
 * summary counts, an unreachable host's first failure, cancelling, the newer-inventory notice and
 * a resumed subscription. Against the fake backend (tests/support/fake-runs.ts).
 */
import { Close, expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { HEAD, WORKSPACE } from '../../support/fake-repos'
import { FakeRuns, runInfo, stats } from '../../support/fake-runs'

function operator(backend: FakeBackend): FakeRuns {
  backend.me = fixtures.me({
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'operator', profiles: [] }],
  })
  return new FakeRuns(backend)
}

test('the run list opens a run whose log streams live', async ({ page, backend }) => {
  const runs = operator(backend)
  runs.add(runInfo({ id: 'run-0001', job_name: 'db', state: 'succeeded', ended_at: '2026-10-27T11:00:00Z' }))
  runs.add(runInfo({ id: 'run-0002', job_name: 'web', state: 'running' }), [
    { host: 'h1', name: 'web-1', state: 'running', refusal: null, started_at: null, ended_at: null, stats: null, first_failure: null },
    { host: 'h2', name: 'web-2', state: 'running', refusal: null, started_at: null, ended_at: null, stats: null, first_failure: null },
  ])
  runs.log('run-0002', 'PLAY [web] *****\n')

  await page.goto('/runs')
  await expect(page.getByRole('heading', { name: 'Runs', level: 1 })).toBeVisible()
  const list = page.getByRole('table', { name: 'Runs' })
  await expect(list.locator('tbody tr')).toHaveCount(2)
  await expect(list.locator('tr[data-run-state="succeeded"]')).toContainText('db')
  await expect(list.locator('tr[data-run-state="running"]')).toContainText(HEAD.slice(0, 12))

  await list.getByRole('link', { name: 'web' }).click()
  await expect(page).toHaveURL(/\/runs\/run-0002$/)
  await expect(page.getByRole('heading', { name: 'Run of web', level: 1 })).toBeVisible()
  const sub = await backend.waitForCmd('SubscribeRun')
  expect(sub.data).toEqual({ run: 'run-0002', from_seq: 0 })

  // History first, then live output as it arrives.
  const log = page.getByRole('log', { name: 'Run output' })
  await expect(log).toContainText('PLAY [web]')
  runs.event('run-0002', 'task_start', 'web-1', 'install nginx')
  runs.log('run-0002', 'TASK [install nginx] *****\nok: [web-1]\n')
  await expect(log).toContainText('ok: [web-1]')

  // The run ends: the state changes live and the host table fills in.
  runs.finish('run-0002', 'succeeded', null, [
    { host: 'h1', name: 'web-1', state: 'succeeded', refusal: null, started_at: null, ended_at: null, stats: stats({ ok: 4, changed: 1 }), first_failure: null },
    { host: 'h2', name: 'web-2', state: 'succeeded', refusal: null, started_at: null, ended_at: null, stats: stats({ ok: 4 }), first_failure: null },
  ])
  await expect(page.locator('[data-run-state]').first()).toHaveAttribute('data-run-state', 'succeeded')
  const hosts = page.getByRole('table', { name: 'Hosts' })
  await expect(hosts.locator('tr[data-host-state="succeeded"]')).toHaveCount(2)

  // Back to the list: leaving the run unsubscribes.
  await page.getByRole('link', { name: 'All runs' }).click()
  await expect(page.getByRole('table', { name: 'Runs' })).toBeVisible()
  expect((await backend.waitForCmd('UnsubscribeRun')).data).toEqual({ run: 'run-0002' })
})

test('per-host results show summary counts and an unreachable host’s first failure', async ({ page, backend }) => {
  const runs = operator(backend)
  runs.add(
    runInfo({
      id: 'run-0003',
      state: 'failed',
      ended_at: '2026-10-27T12:01:00Z',
      summary: {
        hosts: 3,
        succeeded: 1,
        failed: 1,
        refused: 0,
        lost: 1,
        cancelled: 0,
        stats: stats({ ok: 5, changed: 2, failed: 0, unreachable: 1 }),
      },
      hosts: ['web-1', 'web-2', 'web-3'],
    }),
    [
      { host: 'h1', name: 'web-1', state: 'succeeded', refusal: null, started_at: null, ended_at: null, stats: stats({ ok: 5, changed: 2 }), first_failure: null },
      {
        host: 'h2',
        name: 'web-2',
        state: 'failed',
        refusal: null,
        started_at: null,
        ended_at: null,
        stats: stats({ unreachable: 1 }),
        first_failure: {
          task: 'Gathering Facts',
          module: null,
          message: 'ssh: connect to host web-2 port 22: No route to host',
          rc: 255,
          unreachable: true,
        },
      },
      { host: 'h3', name: 'web-3', state: 'lost', refusal: null, started_at: null, ended_at: null, stats: null, first_failure: null },
    ],
  )

  await page.goto('/runs/run-0003')
  const summary = page.getByRole('list', { name: 'Summary' })
  await expect(summary.locator('[data-count="succeeded"]')).toContainText('1')
  await expect(summary.locator('[data-count="failed"]')).toContainText('1')
  await expect(summary.locator('[data-count="lost"]')).toContainText('1')

  const hosts = page.getByRole('table', { name: 'Hosts' })
  const failed = hosts.locator('tr[data-host-state="failed"]')
  await expect(failed).toContainText('web-2')
  await expect(failed).toContainText('unreachable')
  await expect(failed).toContainText('Gathering Facts')
  await expect(failed).toContainText('No route to host')
  await expect(hosts.locator('tr[data-host-state="lost"]')).toContainText('web-3')
  await expect(hosts.locator('tr[data-host-state="succeeded"]')).toContainText('ok 5')
  // A finished run has nothing to cancel.
  await expect(page.getByRole('button', { name: 'Cancel run' })).toHaveCount(0)
})

test('a running run is cancelled after its preview is confirmed', async ({ page, backend }) => {
  const runs = operator(backend)
  runs.add(runInfo({ id: 'run-0004', state: 'running' }))
  await page.goto('/runs/run-0004')
  await page.getByRole('button', { name: 'Cancel run' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Cancel run run-0004 of job web.')
  await expect(dialog.getByRole('list', { name: 'Affected' })).toContainText('web-2')
  expect(runs.runs[0]?.state).toBe('running')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toBeHidden()
  await expect(page.locator('[data-run-state]').first()).toHaveAttribute('data-run-state', 'cancelled')
  await expect(page.getByText('cancelled by user')).toBeVisible()
})

test('a run pinned to an older inventory revision says a newer one exists', async ({ page, backend }) => {
  const runs = operator(backend)
  runs.add(runInfo({ id: 'run-0005', state: 'succeeded', newer_inventory_revision: true }))
  runs.add(runInfo({ id: 'run-0006', state: 'succeeded' }))
  await page.goto('/runs/run-0005')
  await expect(page.getByRole('note', { name: 'Newer inventory' })).toContainText('newer revision of inventory prod')
  await page.goto('/runs/run-0006')
  await expect(page.getByRole('heading', { name: 'Run of web', level: 1 })).toBeVisible()
  await expect(page.getByRole('note', { name: 'Newer inventory' })).toHaveCount(0)
})

test('a dropped socket resubscribes from the last seq it saw, without gaps', async ({ page, backend }) => {
  const runs = operator(backend)
  runs.add(runInfo({ id: 'run-0007', state: 'running' }))
  runs.log('run-0007', 'line one\n')
  await page.goto('/runs/run-0007')
  const log = page.getByRole('log', { name: 'Run output' })
  await expect(log).toContainText('line one')
  runs.log('run-0007', 'line two\n')
  await expect(log).toContainText('line two')

  // Output while the socket is away is stored, not pushed; the resubscribe replays it.
  backend.closeSockets(Close.GoingAway)
  runs.store({ type: 'RunLog', data: { run: 'run-0007', seq: 3, stream: 'stdout', text: 'line three\n' } })
  await expect(log).toContainText('line three')
  const resub = runs.subscribes.at(-1)
  expect(resub).toEqual({ run: 'run-0007', from_seq: 2 })
  await expect(log.getByText('line two')).toHaveCount(1)
})

test('without a workspace the run list says so', async ({ page }) => {
  await page.goto('/runs')
  await expect(page.getByRole('heading', { name: 'Runs', level: 1 })).toBeVisible()
  await expect(page.getByText('You have no workspace yet.')).toBeVisible()
})
