import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

import { refreshCmd, selectCmd, SECTION } from '@/protocol/commands'

const root = join(__dirname, '../../../src')

function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? files(join(dir, e.name)) : [join(dir, e.name)],
  )
}

describe('linter is read-only (FR-003, SC-005)', () => {
  it('never references a mutating command or store action', () => {
    const src = [...files(join(root, 'linter')), join(root, 'components/panels/LinterPanel.tsx')]
      .map((f) => readFileSync(f, 'utf8'))
      .join('\n')
    for (const banned of [
      'Delete',
      'Apply',
      'Patch',
      'Scale',
      'Restart',
      'Cordon',
      'Drain',
      'Uninstall',
      'Rollback',
      'send(',
    ]) {
      expect(src).not.toContain(banned)
    }
  })
  it('command wiring resolves only Refresh*/Select* commands for the new sections and kinds', () => {
    for (const s of [
      SECTION.configmaps,
      SECTION.secrets,
      SECTION.poddisruptionbudgets,
      SECTION.replicasets,
      SECTION.jobs,
      SECTION.cronjobs,
    ]) {
      expect(refreshCmd(s)?.type).toMatch(/^Refresh/)
    }
    for (const k of ['StatefulSet', 'DaemonSet', 'ReplicaSet', 'PodDisruptionBudget']) {
      expect(selectCmd(k, 'd', 'n')?.type).toMatch(/^Select/)
    }
  })
})
