/**
 * US5 audit log (T070): every kind of entry shows who, when, which stack, what ran and how it
 * ended (FR-035); filters narrow the query; export downloads JSONL; nothing edits or deletes an
 * entry (FR-036).
 */
import type { Page } from '@playwright/test'

import type { AuditAction, AuditEntry } from '../../../src/protocol/generated'
import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const ADA = { provider: 'github' as const, subject: '1001', username: 'ada' }
const BOB = { provider: 'discord' as const, subject: '2002', username: 'bob' }

const ACTIONS: AuditAction[] = [
  'up', 'stop', 'restart', 'down', 'pull', 'update', 'save', 'create', 'delete',
  'service_up', 'service_stop', 'service_restart', 'sign_in', 'sign_out', 'denied',
  'shell_open', 'reveal_env', 'user_added', 'user_changed', 'user_removed',
]

function entry(i: number, action: AuditAction, over: Partial<AuditEntry> = {}): AuditEntry {
  return {
    at: new Date(Date.UTC(2026, 9, 20, 9, i)).toISOString(),
    actor: i % 2 ? BOB : ADA,
    action,
    stack: action.startsWith('user_') || action.startsWith('sign_') ? null : i % 3 ? 'web' : 'db',
    preview_id: null,
    operation_id: null,
    argv: ['up', 'stop', 'restart', 'down', 'pull', 'delete'].includes(action)
      ? [['docker', 'compose', '-p', 'web', action, ...(action === 'down' ? ['--volumes'] : [])]]
      : [],
    target: action.startsWith('user_')
      ? { id: 'github:3003', before: action === 'user_added' ? null : { role: 'user', stacks: ['web'] }, after: action === 'user_removed' ? null : { role: 'user', stacks: ['web', 'db'] } }
      : null,
    diff_summary: action === 'save' ? 'compose.yaml +2 -1; .env: DB_PASSWORD changed' : null,
    outcome: { kind: 'succeeded' },
    source_ip: '10.0.0.5',
    ...over,
  }
}

function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList([f.stack('web'), f.stack('db')])
  backend.audit = [
    ...ACTIONS.map((a, i) => entry(i, a)),
    entry(30, 'pull', { outcome: { kind: 'failed', exit_code: 18, stderr_tail: 'manifest unknown' } }),
    entry(31, 'denied', { stack: 'db', outcome: { kind: 'denied', reason: 'not_permitted' } }),
  ]
}

const rows = (page: Page) => page.locator('[data-audit-row]')

test('every kind of entry shows actor, time, stack, what ran and the outcome', async ({ page, backend }) => {
  seed(backend)
  await page.setViewportSize({ width: 1400, height: 2400 })
  await page.goto('/admin/audit')
  await expect(page.getByTestId('audit-count')).toHaveText('22 entries')
  await expect(rows(page)).toHaveCount(22)

  for (const a of ACTIONS) await expect(page.locator(`[data-audit-row][data-action="${a}"]`).first()).toBeAttached()

  // Newest first.
  await expect(rows(page).first()).toHaveAttribute('data-action', 'denied')
  await expect(rows(page).first()).toContainText('Denied: not_permitted')
  await expect(rows(page).first()).toContainText('bob (discord)')

  const failed = page.locator('[data-audit-row][data-outcome="failed"]')
  await expect(failed).toContainText('Failed (exit 18)')
  await expect(failed).toContainText('manifest unknown')

  const down = page.locator('[data-audit-row][data-action="down"]')
  await expect(down).toContainText('docker compose -p web down --volumes')
  await expect(down.locator('time')).toHaveAttribute('datetime', '2026-10-20T09:03:00.000Z')

  await expect(page.locator('[data-audit-row][data-action="save"]')).toContainText('.env: DB_PASSWORD changed')
  await expect(page.locator('[data-audit-row][data-action="user_changed"]')).toContainText(
    'github:3003: user on web → user on web, db',
  )
  await expect(page.locator('[data-audit-row][data-action="user_removed"]')).toContainText('→ none')

  // Read-only: no control inside the log, and the only audit command is the read.
  await expect(page.getByRole('table', { name: 'Audit entries' }).locator('button, input, select, textarea')).toHaveCount(0)
  expect(backend.received.filter((c) => /Audit/.test(c.type)).map((c) => c.type)).toEqual(['GetAudit'])
})

test('filters by stack, actor, action and time', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/admin/audit')
  await expect(rows(page).first()).toBeVisible()
  await backend.waitForCmd('GetAudit')

  await page.getByRole('combobox', { name: 'Action' }).selectOption('pull')
  expect((await backend.waitForCmd('GetAudit')).data.filter).toEqual({ action: 'pull' })
  await expect(rows(page)).toHaveCount(2)

  await page.getByRole('combobox', { name: 'Stack' }).fill('db')
  await page.getByRole('combobox', { name: 'Stack' }).press('Enter')
  expect((await backend.waitForCmd('GetAudit')).data.filter).toEqual({ action: 'pull', stack: 'db' })
  await expect(rows(page)).toHaveCount(1)
  await expect(rows(page)).toHaveAttribute('data-outcome', 'failed')

  await page.getByRole('button', { name: 'Clear' }).click()
  expect((await backend.waitForCmd('GetAudit')).data.filter).toEqual({})
  await expect(rows(page).first()).toBeVisible()

  await page.getByRole('textbox', { name: 'Actor id' }).fill('discord:2002')
  await page.getByRole('button', { name: 'Apply' }).click()
  expect((await backend.waitForCmd('GetAudit')).data.filter).toEqual({ actor: 'discord:2002' })
  for (const r of await rows(page).all()) await expect(r).toContainText('bob')

  // A time window, in the viewer's zone, sent as UTC.
  await page.getByRole('button', { name: 'Clear' }).click()
  await backend.waitForCmd('GetAudit')
  const local = (iso: string): string => {
    const d = new Date(iso)
    const p = (n: number) => String(n).padStart(2, '0')
    return `${String(d.getFullYear())}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`
  }
  await page.getByLabel('From', { exact: true }).fill(local('2026-10-20T09:02:00Z'))
  expect((await backend.waitForCmd('GetAudit')).data.filter).toEqual({ from: '2026-10-20T09:02:00.000Z' })
  await page.getByLabel('To', { exact: true }).fill(local('2026-10-20T09:04:00Z'))
  expect((await backend.waitForCmd('GetAudit')).data.filter).toEqual({ from: '2026-10-20T09:02:00.000Z', to: '2026-10-20T09:04:00.000Z' })
  await expect(rows(page)).toHaveCount(3)
})

test('export downloads the filtered log as JSONL', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/admin/audit')
  await page.getByRole('combobox', { name: 'Action' }).selectOption('pull')
  await expect(rows(page)).toHaveCount(2)

  const link = page.getByRole('link', { name: 'Export JSONL' })
  await expect(link).toHaveAttribute('href', '/api/v1/audit/export?action=pull')
  const [download] = await Promise.all([page.waitForEvent('download'), link.click()])
  expect(download.suggestedFilename()).toBe('depths-audit.jsonl')
  // Playwright cancels a download whose response a route fulfilled, so read the same URL in-page.
  const text = await page.evaluate(async (href) => {
    const res = await fetch(href, { credentials: 'same-origin' })
    return `${res.headers.get('content-type') ?? ''}\n${await res.text()}`
  }, '/api/v1/audit/export?action=pull')
  const [type, ...body] = text.split('\n')
  expect(type).toBe('application/x-ndjson')
  const lines = body.join('\n').trimEnd().split('\n').map((l) => JSON.parse(l) as AuditEntry)
  expect(lines.map((e) => e.action)).toEqual(['pull', 'pull'])
  expect(backend.http.some((h) => h.method === 'GET' && h.path === '/api/v1/audit/export')).toBe(true)
})

test('pages load as the end scrolls into view', async ({ page, backend }) => {
  // A page well past the prefetch window, so the first page holds until the end scrolls near.
  backend.stacks = f.stackList([f.stack('web')])
  backend.audit = Array.from({ length: 100 }, (_, i) => entry(i % 60, ACTIONS[i % ACTIONS.length] ?? 'up'))
  backend.auditPageSize = 40
  await page.setViewportSize({ width: 1280, height: 400 })
  await page.goto('/admin/audit')
  await expect(page.getByTestId('audit-count')).toHaveText('40+ entries')
  expect(backend.received.filter((c) => c.type === 'GetAudit')).toHaveLength(1)
  const scroller = page.getByTestId('audit-scroller')
  await expect(async () => {
    await scroller.evaluate((el) => el.scrollTo(0, el.scrollHeight))
    await expect(page.getByTestId('audit-count')).toHaveText('100 entries', { timeout: 500 })
  }).toPass()
  const cursors = backend.received.flatMap((c) => (c.type === 'GetAudit' && c.data.cursor !== undefined ? [c.data.cursor] : []))
  expect(cursors).toEqual(['40', '80'])
})

test('narrow screens show each entry as a labelled card', async ({ page, backend }) => {
  seed(backend)
  await page.setViewportSize({ width: 390, height: 800 })
  await page.goto('/admin/audit')
  const first = rows(page).first()
  await expect(first).toBeVisible()
  await expect(page.getByRole('columnheader', { name: 'Actor' })).toBeHidden()
  await expect(first.getByText('Outcome', { exact: true })).toBeVisible()
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
  expect(overflow).toBeLessThanOrEqual(0)
})

test('the admin nav reaches both views', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/')
  const nav = page.getByRole('navigation', { name: 'Admin' })
  await nav.getByRole('link', { name: 'Audit' }).click()
  await expect(page.getByRole('heading', { name: 'Audit log' })).toBeVisible()
  await expect(nav.getByRole('link', { name: 'Audit' })).toHaveAttribute('aria-current', 'page')
  await nav.getByRole('link', { name: 'Users' }).click()
  await expect(page.getByRole('heading', { name: 'Users', level: 1 })).toBeVisible()
})
