/**
 * Reviews (spec 055 US8, T102): signed-mode runs held for review, one group of hosts per base.
 *
 * The list follows `Reviews`, the pushed `ReviewRequested` (re-lists) and `Review` (a group that
 * changed). Selecting a group asks for `Review` (its commits with their signature status, the
 * flagged files and the decisions so far) and the first `ReviewDiff` (the changed files); a file's
 * diff is paged, one `ReviewDiff` per page.
 *
 * Approving is an override: `PreviewOverride` (with the required comment) answers a `Preview` and
 * an `OverrideChallenge`. Confirming signs that challenge with a passkey, never a recent step-up:
 * the assertion is over the exact run, commits and hosts, and hosts check it. Denying is
 * `PreviewOverrideDeny`, confirmed like any change.
 *
 * Trusted signers (the early check's list) are a workspace setting kept here too:
 * `ListSigners`, `PreviewSignerAdd` and `PreviewSignerRemove`, which ask for step-up.
 *
 * Replies count only when they answer a command sent from here; a passkey assertion is passed
 * straight to `Confirm` and never kept.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import { signChallenge, usePasskeys } from '@/features/passkeys/store'
import type {
  Cmd,
  DiffFile,
  DiffPage,
  OverrideChallenge,
  Preview,
  RequestId,
  ReviewCommit,
  ReviewDecision,
  ReviewFlag,
  ReviewFlagKind,
  ReviewInfo,
  ReviewState,
  SignatureStatus,
  SignerInfo,
  SignerKind,
} from '@/protocol/wire'

/** The native input and select style the forms use. */
export const FIELD =
  'h-8 border border-input bg-background px-2 text-xs outline-none focus-visible:border-accent-neon'

const OFFLINE = 'Keel is not connected. Wait for it to reconnect, then try again.'

/** The server's limit on a review comment (`ReviewComment`). */
export const MAX_COMMENT = 2000

/** A comment the server accepts: not blank, at most 2000 characters. */
export function commentOk(text: string): boolean {
  // Code points, as the server counts them.
  const n = Array.from(text).length
  return text.trim().length > 0 && n <= MAX_COMMENT
}

/** The first 12 characters of a commit id. */
export function shortSha(sha: string | null): string {
  return sha === null ? 'none' : sha.slice(0, 12)
}

export const STATUS_LABELS: Record<SignatureStatus, string> = {
  good: 'Signed, trusted',
  missing: 'Unsigned',
  unparseable: 'Signature unreadable',
  unknown_key: 'Unknown key',
  bad_signature: 'Bad signature',
  expired: 'Key expired',
  revoked: 'Key revoked',
  not_signing_capable: 'Key cannot sign',
  key_invalid: 'Key invalid',
  wrong_namespace: 'Wrong namespace',
  weak_hash: 'Weak hash',
  unchecked: 'Not checked',
}

export const FLAG_LABELS: Record<ReviewFlagKind, string> = {
  keel_config: 'Keel config (.keel/)',
  inventory: 'Inventory',
  runner_vars: 'keel_runner_* variables',
}

export const REASON_LABELS: Record<ReviewInfo['reason'], string> = {
  unsigned: 'Unsigned commits',
  downgrade: 'Downgrade',
  baseline: 'First signed-mode run (baseline)',
  check_failed: 'Check failed',
}

export interface ReviewDetail {
  review: ReviewInfo
  commits: ReviewCommit[]
  flags: ReviewFlag[]
  decisions: ReviewDecision[]
}

/** A decision being confirmed. */
export type Pending =
  | { kind: 'approve'; preview: Preview; challenge: OverrideChallenge | null }
  | { kind: 'deny'; preview: Preview }
  | { kind: 'signer'; preview: Preview }

export interface ReviewsState {
  workspace: string | null
  filter: ReviewState | null
  items: ReviewInfo[]
  selected: string | null
  detail: ReviewDetail | null
  files: DiffFile[]
  diff: DiffPage | null
  pending: Pending | null
  /** The authenticator is answering an override challenge. */
  signing: boolean
  signers: SignerInfo[]
  error: string | null
  load: (workspace: string) => void
  setFilter: (filter: ReviewState | null) => void
  select: (group: string | null) => void
  openFile: (path: string, page?: number) => void
  approve: (comment: string) => void
  deny: (comment: string) => void
  addSigner: (kind: SignerKind, public_key: string, label: string) => void
  removeSigner: (signer: string) => void
  confirm: () => Promise<void>
  cancel: () => void
  clearError: () => void
}

type Purpose = 'approve' | 'deny' | 'signer' | 'read'
const mine = new Map<RequestId, Purpose>()

function send(cmd: Cmd, purpose: Purpose = 'read'): boolean {
  const id = sendCmd(cmd)
  if (id === null) {
    useReviews.setState({ error: OFFLINE })
    return false
  }
  mine.set(id, purpose)
  return true
}

function list(): void {
  const { workspace, filter } = useReviews.getState()
  if (workspace) send({ type: 'ListReviews', data: { workspace, state: filter } })
}

export const useReviews = create<ReviewsState>()((set, get) => ({
  workspace: null,
  filter: 'open',
  items: [],
  selected: null,
  detail: null,
  files: [],
  diff: null,
  pending: null,
  signing: false,
  signers: [],
  error: null,

  load: (workspace) => {
    if (get().workspace !== workspace) {
      set({ workspace, items: [], selected: null, detail: null, files: [], diff: null, signers: [] })
    }
    list()
    send({ type: 'ListSigners', data: { workspace } })
  },
  setFilter: (filter) => {
    set({ filter })
    list()
  },
  select: (group) => {
    set({ selected: group, detail: null, files: [], diff: null, error: null })
    if (group === null) return
    send({ type: 'GetReview', data: { group } })
    send({ type: 'GetReviewDiff', data: { group, path: null, page: null } })
  },
  openFile: (path, page = 0) => {
    const group = get().selected
    if (!group) return
    send({ type: 'GetReviewDiff', data: { group, path, page } })
  },
  approve: (comment) => {
    const group = get().selected
    if (!group || !commentOk(comment)) return
    set({ error: null, pending: null })
    send({ type: 'PreviewOverride', data: { group, comment } }, 'approve')
  },
  deny: (comment) => {
    const group = get().selected
    if (!group || !commentOk(comment)) return
    set({ error: null, pending: null })
    send({ type: 'PreviewOverrideDeny', data: { group, comment } }, 'deny')
  },
  addSigner: (kind, public_key, label) => {
    const workspace = get().workspace
    if (!workspace) return
    set({ error: null, pending: null })
    send({ type: 'PreviewSignerAdd', data: { workspace, kind, public_key, label, user: null } }, 'signer')
  },
  removeSigner: (signer) => {
    set({ error: null, pending: null })
    send({ type: 'PreviewSignerRemove', data: { signer } }, 'signer')
  },
  confirm: async () => {
    const p = get().pending
    if (!p) return
    const { preview_id } = p.preview
    if (p.kind === 'approve') {
      // An override is confirmed only by a passkey over its own challenge.
      if (!p.challenge) return
      set({ signing: true, error: null })
      try {
        const assertion = await signChallenge(p.challenge.request)
        if (get().pending !== p) return
        send({ type: 'Confirm', data: { preview_id, step_up: assertion } }, 'approve')
      } catch (e) {
        if (get().pending === p) set({ error: e instanceof Error ? e.message : String(e) })
      } finally {
        set({ signing: false })
      }
      return
    }
    const verified = useSession.getState().me?.verification.verified ?? false
    if (p.preview.step_up && !verified) {
      const assertion = await usePasskeys.getState().requestStepUp(p.preview.summary)
      if (!assertion) return
      send({ type: 'Confirm', data: { preview_id, step_up: assertion } }, p.kind)
    } else {
      send({ type: 'Confirm', data: { preview_id } }, p.kind)
    }
  },
  cancel: () => {
    set({ pending: null, signing: false })
  },
  clearError: () => {
    set({ error: null })
  },
}))

function upsert(items: ReviewInfo[], review: ReviewInfo): ReviewInfo[] {
  return items.map((x) => (x.id === review.id ? review : x))
}

onServerEvent(({ reply_to, event }) => {
  const s = useReviews.getState()
  const purpose = reply_to === undefined ? undefined : mine.get(reply_to)
  const ours = purpose !== undefined
  switch (event.type) {
    case 'Reviews':
      if (ours) useReviews.setState({ items: event.data.items })
      break
    case 'ReviewRequested':
      if (event.data.workspace === s.workspace) list()
      break
    case 'Review': {
      const { review, commits, flags, decisions } = event.data
      if (review.workspace !== s.workspace && !ours) break
      const patch: Partial<ReviewsState> = { items: upsert(s.items, review) }
      if (review.id === s.selected) patch.detail = { review, commits, flags, decisions }
      if (ours && (purpose === 'approve' || purpose === 'deny')) patch.pending = null
      useReviews.setState(patch)
      // A decided group may leave the filtered list.
      if (s.filter !== null && review.state !== s.filter && s.items.some((x) => x.id === review.id)) list()
      break
    }
    case 'ReviewDiff':
      if (ours && event.data.group === s.selected) {
        useReviews.setState(
          event.data.page === null ? { files: event.data.files } : { files: event.data.files, diff: event.data.page },
        )
      }
      break
    case 'Signers':
      if (ours && event.data.workspace === s.workspace) {
        useReviews.setState({ signers: event.data.items, ...(purpose === 'signer' ? { pending: null } : {}) })
      }
      break
    case 'Preview':
      if (ours && purpose !== 'read') {
        useReviews.setState({
          pending:
            purpose === 'approve' ? { kind: 'approve', preview: event.data, challenge: null } : { kind: purpose, preview: event.data },
          error: null,
        })
      }
      break
    case 'OverrideChallenge': {
      const p = s.pending
      if (ours && p?.kind === 'approve' && p.preview.preview_id === event.data.preview_id) {
        useReviews.setState({ pending: { ...p, challenge: event.data } })
      }
      break
    }
    case 'Confirmed':
      if (ours && purpose !== 'approve' && purpose !== 'deny') useReviews.setState({ pending: null })
      break
    case 'PasskeyEnrolmentRequired':
      if (ours) {
        useReviews.setState({ error: 'Register a passkey first: an override is signed with it.', pending: null })
      }
      break
    case 'OpError':
      if (ours) useReviews.setState({ error: event.data.message, pending: null })
      break
    case 'Denied':
      if (ours) useReviews.setState({ error: `Not allowed: ${event.data.reason}`, pending: null })
      break
    case 'PasskeyRefused':
      if (ours) useReviews.setState({ error: 'The passkey was refused. Try again.', pending: null })
      break
    default:
      break
  }
})

onReconnected(() => {
  const { workspace, load, selected, select } = useReviews.getState()
  if (workspace) load(workspace)
  if (selected) select(selected)
})
