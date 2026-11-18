/**
 * Set-up for spec 068's run specs (T044, T051, T060, T066): `converging`'s workspace with the run
 * and binding commands of `fake-runs-068.ts`. Not a spec.
 */
import type { Locator, Page } from '@playwright/test'

import type { FakeBackend } from '../../support/fake-backend'
import { FakeRuns068 } from '../../support/fake-runs-068'
import type { Options } from '../s066/repo-setup'
import { converging, press, type Converge } from './setup'

export interface Runs068 extends Converge {
  r068: FakeRuns068
}

export function running(backend: FakeBackend, o: Options = {}): Runs068 {
  const c = converging(backend, o)
  return { ...c, r068: new FakeRuns068(backend, c.runs) }
}

/** Opens job `name` on the Repository page's Jobs tab, then its Start dialog, by keyboard. */
export async function openStart(page: Page, name: string): Promise<Locator> {
  await page.goto(`/repository?tab=jobs&item=${name}`)
  await press(page, page.getByRole('button', { name: 'Run', exact: true }))
  const dialog = page.getByRole('dialog', { name: `Run ${name}` })
  await dialog.waitFor()
  return dialog
}

/** The governance change dialog, once the server's preview is in. */
export function change(page: Page, title: string): Locator {
  return page.getByRole('dialog', { name: title })
}
