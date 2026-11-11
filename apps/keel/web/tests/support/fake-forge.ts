/**
 * Spec 066's forge on the fake backend (T050, research.md R8): what a confirmed change that writes
 * to the repository (`PreviewHostAdd`, `PreviewSopsRecipient`, `PreviewJobPropose`) answers as its
 * `ChangeOutcome`, the way apps/keel/server does:
 *
 * - with a forge connection and `mode: 'opened'`, a pull request on a new branch (`Opened`);
 * - with `mode: 'failed'`, the forge's refusal and the patch (`Failed`);
 * - with no forge connection on the workspace, the patch to apply by hand (`Manual`).
 *
 * `connect(workspace)` adds the connection that `GetForgeConnection` answers, through the
 * workspace repository fake, so the Draft job dialog offers Propose. Every change asked of the
 * forge is kept in `changes`.
 */
import type { ChangeOutcome } from '../../src/protocol/wire'
import { NOW } from './fake-repos'
import type { FakeWorkspaceRepo } from './fake-workspace-repo'

export const FORGE_BASE = 'https://git.example.com'

export class FakeForge {
  /** What a connected forge does with the next change. */
  mode: 'opened' | 'failed' = 'opened'
  /** The refusal `mode: 'failed'` reports. */
  error = 'forge_refused: inventory/hosts.yml changed on main since Keel read it (sha mismatch).'
  /** Every change asked of the forge, in order. */
  readonly changes: { workspace: string; branch: string; title: string; diff: string }[] = []

  private readonly repo: FakeWorkspaceRepo

  constructor(repo: FakeWorkspaceRepo) {
    this.repo = repo
  }

  /** Gives `workspace` a Forgejo connection. */
  connect(workspace: string): this {
    this.repo.forges.set(workspace, {
      workspace,
      provider: 'forgejo',
      api_base: `${FORGE_BASE}/api/v1`,
      credential: 'cred-forge',
      probed_at: NOW,
      probe_error: null,
      created_at: NOW,
    })
    return this
  }

  /** The outcome of proposing `diff` on a new branch named after `slug`. */
  outcome(workspace: string, slug: string, title: string, diff: string): ChangeOutcome {
    if (!this.repo.forges.has(workspace)) {
      return { kind: 'manual', diff, reason: 'This workspace has no forge connection, so Keel cannot open a pull request.' }
    }
    if (this.mode === 'failed') return { kind: 'failed', diff, error: this.error }
    const branch = `keel/${slug}-${String(this.changes.length + 1)}`
    this.changes.push({ workspace, branch, title, diff })
    return { kind: 'opened', url: `${FORGE_BASE}/ops/ansible/pulls/${String(40 + this.changes.length)}`, branch }
  }
}
