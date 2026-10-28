/**
 * The fake backend's reviews and trusted signers (spec 055 US8, T095).
 *
 * An in-memory model answering the review commands as apps/keel/server/src/review.rs does,
 * installed on a `FakeBackend` with `on(...)` so the shared fake stays untouched:
 *
 * - `ListReviews` answers `Reviews` (by workspace and state); `GetReview` answers `Review`.
 * - `GetReviewDiff` answers the changed files, and with a path one page of that file's diff
 *   (`PAGE_LINES` lines a page); a page past the end is `bad_page`, a file not changed `not_changed`.
 * - `PreviewOverride` needs a registered passkey (else `PasskeyEnrolmentRequired`), previews with
 *   step-up and issues an `OverrideChallenge` whose options ask for one of the person's passkeys.
 *   Its `Confirm` needs an assertion from that very ceremony (a recent step-up does not count:
 *   `override_assertion_required`), records the approval and answers `Confirmed` and `Review`.
 * - `PreviewOverrideDeny` previews without step-up; its `Confirm` denies the group.
 * - `ListSigners`, `PreviewSignerAdd` (`signer_exists` for a key already listed) and
 *   `PreviewSignerRemove`, whose `Confirm` needs a fresh check, as the shared fake does.
 */
import type { Cmd, DiffFile, Event, ReviewCommit, ReviewFlag, ReviewInfo, SignerInfo } from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import { NOW, WORKSPACE } from './fake-repos'

const EXPIRES = '2026-10-27T12:15:00Z'
const OVERRIDE_EXPIRES = '2026-10-28T12:00:00Z'
/** Lines on one page of a file's diff (the server's is 400). */
export const PAGE_LINES = 20

export const TARGET = 'c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00'
export const BASE = 'ba5e0000ba5e0000ba5e0000ba5e0000ba5e0000'
export const SIGNER_FP = 'SHA256:cisignerfingerprint00000000000000000000000'

export function reviewInfo(over: Partial<ReviewInfo> = {}): ReviewInfo {
  return {
    id: 'group-0001',
    run: 'run-0001',
    workspace: WORKSPACE,
    binding: 'binding-0001',
    job_name: 'web',
    base: BASE,
    target: TARGET,
    hosts: ['web-1', 'web-2'],
    reason: 'unsigned',
    state: 'open',
    approvals: 0,
    quorum: 1,
    four_eyes: false,
    flagged: 3,
    created_at: NOW,
    ...over,
  }
}

export const COMMITS: ReviewCommit[] = [
  {
    id: TARGET,
    author: 'Grace Hopper <grace@example.org>',
    committer: 'Grace Hopper <grace@example.org>',
    summary: 'Run the web role as the runner user',
    status: 'missing',
    signer: null,
  },
  {
    id: 'feed0000feed0000feed0000feed0000feed0000',
    author: 'CI <ci@example.org>',
    committer: 'CI <ci@example.org>',
    summary: 'Add the web group vars',
    status: 'good',
    signer: SIGNER_FP,
  },
  {
    id: 'bad00000bad00000bad00000bad00000bad00000',
    author: 'Mallory <mallory@example.org>',
    committer: 'Mallory <mallory@example.org>',
    summary: 'Tweak the job manifest',
    status: 'unknown_key',
    signer: null,
  },
]

export const FLAGS: ReviewFlag[] = [
  { kind: 'keel_config', path: '.keel/jobs.yaml' },
  { kind: 'inventory', path: 'inventories/prod/group_vars/web.yml' },
  { kind: 'runner_vars', path: 'playbooks/web.yaml' },
]

const FILES: DiffFile[] = [
  { path: '.keel/jobs.yaml', status: 'modified', flags: ['keel_config'] },
  { path: 'inventories/prod/group_vars/web.yml', status: 'added', flags: ['inventory'] },
  { path: 'playbooks/web.yaml', status: 'modified', flags: ['runner_vars'] },
  { path: 'roles/web/files/logo.png', status: 'added', flags: [] },
]

/** Each file's unified diff lines. The playbook spans two pages. */
function diffLines(path: string): string[] {
  if (path === 'playbooks/web.yaml') {
    const body = Array.from({ length: 30 }, (_, i) => ` - task ${String(i + 1)}`)
    return ['@@ -1,30 +1,31 @@', '+  vars:', '+    keel_runner_user: root', ...body]
  }
  if (path === '.keel/jobs.yaml') return ['@@ -3,2 +3,2 @@', '-    paths: [playbooks/**]', '+    paths: ["**"]']
  return ['@@ -0,0 +1 @@', '+tier: front']
}

type Pending =
  | { kind: 'approve'; group: string; comment: string; ceremony: string }
  | { kind: 'deny'; group: string; comment: string }
  | { kind: 'signer_add'; signer: SignerInfo; key: string }
  | { kind: 'signer_remove'; signer: string }

export class FakeReviews {
  reviews: ReviewInfo[] = [reviewInfo()]
  signers: SignerInfo[] = []
  /** Every decision confirmed, in order, with the passkey credential that signed an approval. */
  readonly decisions: { group: string; approve: boolean; comment: string; credential: string | null }[] = []
  private readonly keys = new Map<string, string>()
  private readonly pending = new Map<string, Pending>()
  private seq = 0

  private readonly backend: FakeBackend

  constructor(backend: FakeBackend) {
    this.backend = backend
    const types = [
      'ListReviews',
      'GetReview',
      'GetReviewDiff',
      'PreviewOverride',
      'PreviewOverrideDeny',
      'ListSigners',
      'PreviewSignerAdd',
      'PreviewSignerRemove',
      'Confirm',
    ] as const
    for (const type of types) backend.on(type, (cmd) => this.respond(cmd))
  }

  private reviewEvent(id: string): Event[] {
    const review = this.reviews.find((r) => r.id === id)
    if (!review) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such review.' } }]
    return [
      {
        type: 'Review',
        data: {
          review,
          commits: COMMITS,
          flags: FLAGS,
          decisions: this.decisions
            .filter((d) => d.group === id)
            .map((d) => ({
              approver: 'ada@example.com',
              approve: d.approve,
              comment: d.comment,
              at: NOW,
              expires_at: d.approve ? OVERRIDE_EXPIRES : null,
            })),
        },
      },
    ]
  }

  private preview(action: string, summary: string, affected: string[], step_up: boolean, p: Pending): Event {
    const preview_id = `review-preview-${String(++this.seq)}`
    this.pending.set(preview_id, p)
    return { type: 'Preview', data: { preview_id, action, summary, affected, step_up, expires_at: EXPIRES } }
  }

  private open(group: string): ReviewInfo | Event[] {
    const review = this.reviews.find((r) => r.id === group)
    if (!review) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such review.' } }]
    if (review.state !== 'open') {
      return [{ type: 'OpError', data: { code: 'review_closed', message: 'That review is already decided.' } }]
    }
    return review
  }

  private decide(group: string, state: ReviewInfo['state'], approvals: number): void {
    this.reviews = this.reviews.map((r) => (r.id === group ? { ...r, state, approvals } : r))
  }

  private respond(cmd: Cmd): Event[] | undefined {
    switch (cmd.type) {
      case 'ListReviews': {
        const { workspace, state } = cmd.data
        const items = this.reviews.filter((r) => (workspace === null || r.workspace === workspace) && (state === null || r.state === state))
        return [{ type: 'Reviews', data: { items } }]
      }
      case 'GetReview':
        return this.reviewEvent(cmd.data.group)
      case 'GetReviewDiff': {
        const { group, path, page } = cmd.data
        if (!this.reviews.some((r) => r.id === group)) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such review.' } }]
        if (path === null) return [{ type: 'ReviewDiff', data: { group, files: FILES, page: null } }]
        if (!FILES.some((f) => f.path === path)) {
          return [{ type: 'OpError', data: { code: 'not_changed', message: `${path} does not change in this review.` } }]
        }
        const binary = path.endsWith('.png')
        const lines = binary ? [] : diffLines(path)
        const pages = Math.max(1, Math.ceil(lines.length / PAGE_LINES))
        const n = page ?? 0
        if (n >= pages) return [{ type: 'OpError', data: { code: 'bad_page', message: 'No such page.' } }]
        return [
          {
            type: 'ReviewDiff',
            data: {
              group,
              files: FILES,
              page: { path, page: n, pages, lines: lines.slice(n * PAGE_LINES, (n + 1) * PAGE_LINES), binary },
            },
          },
        ]
      }
      case 'PreviewOverride': {
        const review = this.open(cmd.data.group)
        if (Array.isArray(review)) return review
        if (this.backend.passkeys.length === 0) return [{ type: 'PasskeyEnrolmentRequired' }]
        const ceremony = `override-cer-${String(++this.seq)}`
        const preview = this.preview(
          'preview_override',
          `Override the review of ${review.job_name} at ${review.target.slice(0, 12)} on ${review.hosts.join(', ')}.`,
          review.hosts,
          true,
          { kind: 'approve', group: review.id, comment: cmd.data.comment, ceremony },
        )
        const preview_id = preview.type === 'Preview' ? preview.data.preview_id : ''
        const canonical = JSON.stringify({
          commits: COMMITS.map((c) => c.id),
          expires_at: OVERRIDE_EXPIRES,
          hosts: review.hosts,
          run: review.run,
          target: review.target,
        })
        return [
          preview,
          {
            type: 'OverrideChallenge',
            data: {
              preview_id,
              request: {
                ceremony,
                options: {
                  publicKey: {
                    challenge: Buffer.from(`override-${canonical}`, 'utf8').toString('base64url'),
                    rpId: 'localhost',
                    userVerification: 'required',
                    allowCredentials: this.backend.passkeys.map((p) => ({ type: 'public-key', id: p.id })),
                    timeout: 60_000,
                  },
                },
              },
              canonical,
              expires_at: OVERRIDE_EXPIRES,
            },
          },
        ]
      }
      case 'PreviewOverrideDeny': {
        const review = this.open(cmd.data.group)
        if (Array.isArray(review)) return review
        return [
          this.preview('preview_override_deny', `Deny the review of ${review.job_name}: its run is denied.`, review.hosts, false, {
            kind: 'deny',
            group: review.id,
            comment: cmd.data.comment,
          }),
        ]
      }
      case 'ListSigners':
        return [{ type: 'Signers', data: { workspace: cmd.data.workspace, items: this.signers } }]
      case 'PreviewSignerAdd': {
        const { public_key, label, kind } = cmd.data
        if (kind === 'ssh' && !public_key.startsWith('ssh-')) {
          return [{ type: 'OpError', data: { code: 'invalid_signer_key', message: 'That is not an SSH public key.' } }]
        }
        if ([...this.keys.values()].includes(public_key)) {
          return [{ type: 'OpError', data: { code: 'signer_exists', message: 'That key is already a trusted signer.' } }]
        }
        const signer: SignerInfo = {
          id: `signer-${String(++this.seq)}`,
          kind,
          fingerprint: SIGNER_FP,
          label,
          user: null,
          added_by: 'ada@example.com',
          added_at: NOW,
        }
        return [this.preview('preview_signer_add', `Trust ${label} (${SIGNER_FP}) to sign commits.`, [label], true, { kind: 'signer_add', signer, key: public_key })]
      }
      case 'PreviewSignerRemove': {
        const signer = this.signers.find((s) => s.id === cmd.data.signer)
        if (!signer) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such signer.' } }]
        return [this.preview('preview_signer_remove', `Stop trusting ${signer.label}.`, [signer.label], true, { kind: 'signer_remove', signer: signer.id })]
      }
      case 'Confirm': {
        const { preview_id, step_up } = cmd.data
        const p = this.pending.get(preview_id)
        if (p === undefined) return undefined
        switch (p.kind) {
          case 'approve': {
            // Only an assertion over this override's own challenge confirms it.
            if (step_up?.ceremony !== p.ceremony || !this.backend.passkeys.some((k) => k.id === step_up.credential.id)) {
              return [
                {
                  type: 'OpError',
                  data: { code: 'override_assertion_required', message: 'Sign the override with your passkey.' },
                },
              ]
            }
            this.pending.delete(preview_id)
            this.decisions.push({ group: p.group, approve: true, comment: p.comment, credential: step_up.credential.id })
            this.decide(p.group, 'approved', 1)
            return [{ type: 'Confirmed', data: { preview_id } }, ...this.reviewEvent(p.group)]
          }
          case 'deny':
            this.pending.delete(preview_id)
            this.decisions.push({ group: p.group, approve: false, comment: p.comment, credential: null })
            this.decide(p.group, 'denied', 0)
            return [{ type: 'Confirmed', data: { preview_id } }, ...this.reviewEvent(p.group)]
          default:
            break
        }
        if (!step_up && !this.backend.me?.verification.verified) {
          return [{ type: 'OpError', data: { code: 'step_up_required', message: 'Confirm it’s you with a passkey first.' } }]
        }
        this.pending.delete(preview_id)
        if (p.kind === 'signer_add') {
          this.signers = [...this.signers, p.signer]
          this.keys.set(p.signer.id, p.key)
        } else {
          this.signers = this.signers.filter((s) => s.id !== p.signer)
          this.keys.delete(p.signer)
        }
        return [
          { type: 'Confirmed', data: { preview_id } },
          { type: 'Signers', data: { workspace: WORKSPACE, items: this.signers } },
        ]
      }
      default:
        return undefined
    }
  }
}
