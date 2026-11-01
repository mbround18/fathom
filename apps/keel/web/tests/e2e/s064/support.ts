/**
 * Shared set-up for spec 064's specs: sign in with a role in the ops workspace, held runs in the
 * runs model (tests/support/fake-runs.ts), and the page's clock fixed at the fixtures' `NOW`, so
 * "Window closes tomorrow" reads the same on every run.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, type FakeBackend } from '../../support/fake-backend'
import { NOW, WORKSPACE } from '../../support/fake-repos'
import { FakeRuns } from '../../support/fake-runs'
import type { Me, TeamRole } from '../../../src/protocol/wire'

/** Signs in with `role` on the whole ops workspace, verified unless `over` says otherwise. */
export function signIn(backend: FakeBackend, role: TeamRole | null, over: Partial<Me> = {}): FakeRuns {
  backend.me = fixtures.me({
    instance_role: 'user',
    grants: role ? [{ workspace: WORKSPACE, workspace_slug: 'ops', role, profiles: [] }] : [],
    verification: fixtures.verification({ enrolled: true, verified: true }),
    ...over,
  })
  return new FakeRuns(backend)
}

export async function freezeClock(page: Page): Promise<void> {
  await page.clock.setFixedTime(new Date(NOW))
}

/** Opens a run's page and waits for its approval panel. */
export async function openRun(page: Page, id: string) {
  await page.goto(`/runs/${id}`)
  const panel = page.locator('[data-approval-panel]')
  await expect(panel).toBeVisible()
  return panel
}

/** The approve or deny dialog. */
export function decision(page: Page, kind: 'approve' | 'deny') {
  return page.getByRole('dialog', { name: kind === 'approve' ? 'Approve this run' : 'Deny this run' })
}
