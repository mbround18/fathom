/**
 * Shared set-up for spec 068's specs: the workspace `ops` with its primary repository bound (spec
 * 066's set-up), three hosts, runs, and the converge fake. Not a spec.
 */
import type { Page } from '@playwright/test'

import type { HostInfo } from '../../../src/protocol/wire'
import type { FakeBackend } from '../../support/fake-backend'
import { FakeConverge } from '../../support/fake-converge'
import { hostInfo } from '../../support/fake-hosts'
import { FakeRuns } from '../../support/fake-runs'
import { setUp, type Options, type Repo } from '../s066/repo-setup'

export interface Converge extends Repo {
  runs: FakeRuns
  converge: FakeConverge
}

export const WIDTHS = [
  { name: 'phone', width: 360, height: 800 },
  { name: 'desktop', width: 1280, height: 900 },
] as const

export function converging(backend: FakeBackend, o: Options = {}, hosts?: (ws: string) => HostInfo[]): Converge {
  const repo = setUp(backend, { role: 'operator', ...o })
  const ws = repo.ops.id
  repo.hosts.hosts = hosts?.(ws) ?? [
    hostInfo({ id: 'host-alpha', workspace: ws, inventory_name: 'alpha', state: 'online', profiles: ['base', 'keel-node'] }),
    hostInfo({ id: 'host-bravo', workspace: ws, inventory_name: 'bravo', state: 'online', profiles: ['base', 'keel-node'] }),
    hostInfo({ id: 'host-charlie', workspace: ws, inventory_name: 'charlie', state: 'online', profiles: ['base'] }),
  ]
  const runs = new FakeRuns(backend)
  const converge = new FakeConverge(backend, runs, repo.hosts, ws)
  return { ...repo, runs, converge }
}

/** Focuses `name`'s control and presses `key`: the keyboard path, without counting tabs. */
export async function press(page: Page, locator: ReturnType<Page['locator']>, key = 'Enter'): Promise<void> {
  await locator.focus()
  await page.keyboard.press(key)
}
