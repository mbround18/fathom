/**
 * Governance state (spec 055 US6 T038): workspaces, teams, users and API tokens, and the one
 * preview → confirm flow every change to them goes through.
 *
 * - `load(kind)` asks for a list through `loadList`. A refusal (`Denied`, `OpError`, or Keel not
 *   connected) is kept per list and shown as the server sent it; the page never decides on its
 *   own who may see what.
 * - `propose(title, cmd)` sends a `Preview*` command. The reply is a `Preview` (summary, the
 *   affected users and objects, whether it needs a passkey) or a refusal, shown verbatim.
 * - `confirm()` sends `Confirm`. A preview that needs step-up asks the passkey store for a fresh
 *   assertion first, unless this session verified recently. `Confirmed` reloads the lists on show;
 *   `ApiTokenCreated` carries a token's secret, which is kept in memory only until the dialog
 *   closes and never written anywhere else.
 *
 * Replies are matched on `reply_to`; a reply for a change that was dismissed is ignored.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { loadList, OFFLINE, refusalOf as listRefusalOf, type Refusal } from '@/app/load'
import { useSession } from '@/app/session-store'
import { usePasskeys } from '@/features/passkeys/store'
import type {
  ApiTokenInfo,
  Cmd,
  Event,
  PasskeyAssertion,
  Preview,
  RequestId,
  TeamInfo,
  TokenSecret,
  UserInfo,
  WorkspaceInfo,
} from '@/protocol/wire'

export type ListKind = 'workspaces' | 'teams' | 'users' | 'tokens'

const LIST_CMD: Record<ListKind, Cmd> = {
  workspaces: { type: 'ListWorkspaces' },
  teams: { type: 'ListTeams' },
  users: { type: 'ListUsers' },
  tokens: { type: 'ListApiTokens' },
}

export type { Refusal } from '@/app/load'

export type ChangePhase = 'asking' | 'preview' | 'stepping' | 'confirming' | 'refused' | 'done'

export interface Change {
  /** What the person asked for, as the dialog's title. */
  title: string
  phase: ChangePhase
  preview: Preview | null
  refusal: Refusal | null
  /** A new API token and its secret, shown once. */
  created: { token: ApiTokenInfo; secret: TokenSecret } | null
}

export interface GovernanceState {
  workspaces: WorkspaceInfo[] | null
  teams: TeamInfo[] | null
  users: UserInfo[] | null
  tokens: ApiTokenInfo[] | null
  refused: Partial<Record<ListKind, Refusal>>
  change: Change | null
  load: (kind: ListKind) => void
  /**
   * `onResult` sees every event answering the change's `Confirm` (`Confirmed` first, then the
   * result, or a refusal), for callers that keep what the change returns (spec 060 settings).
   */
  propose: (title: string, cmd: Cmd, onResult?: (event: Event) => void) => void
  confirm: () => Promise<void>
  dismiss: () => void
}

const ENROL_FIRST: Refusal = {
  kind: 'error',
  code: 'passkey_required',
  message: 'This change needs a passkey. Register one, then confirm again.',
}

/** Handlers for replies, by request id. */
const replies = new Map<RequestId, (event: Event) => void>()
/** Request ids that belong to the open change, dropped when it closes. */
let changeIds: RequestId[] = []
/** Bumped whenever a change opens or closes, so stale replies do nothing. */
let generation = 0
/** The open change's `onResult`. */
let resultHandler: ((event: Event) => void) | undefined

onServerEvent(({ reply_to, event }) => {
  if (reply_to === undefined) return
  replies.get(reply_to)?.(event)
})

onReconnected(() => {
  const s = useGovernance.getState()
  for (const kind of loadedKinds(s)) s.load(kind)
})

function refusalOf(event: Event): Refusal | null {
  if (event.type === 'PasskeyEnrolmentRequired') return ENROL_FIRST
  return listRefusalOf(event)
}

/** The list `kind` asked for, as a patch, when `event` is it. */
function listOf(kind: ListKind, event: Event): Partial<GovernanceState> | undefined {
  if (event.type === 'Workspaces' && kind === 'workspaces') return { workspaces: event.data.items }
  if (event.type === 'Teams' && kind === 'teams') return { teams: event.data.items }
  if (event.type === 'Users' && kind === 'users') return { users: event.data.items }
  if (event.type === 'ApiTokens' && kind === 'tokens') return { tokens: event.data.items }
  return undefined
}

function loadedKinds(s: GovernanceState): ListKind[] {
  return (Object.keys(LIST_CMD) as ListKind[]).filter((k) => s[k] !== null || s.refused[k] !== undefined)
}

/** Sends `cmd` for the open change; `null` when offline. */
function sendForChange(cmd: Cmd, gen: number, handle: (event: Event) => void): RequestId | null {
  const id = sendCmd(cmd)
  if (id === null) return null
  changeIds.push(id)
  replies.set(id, (event) => {
    if (gen === generation) handle(event)
  })
  return id
}

/** The session verified within its window, so `Confirm` needs no fresh assertion. */
function recentlyVerified(): boolean {
  return useSession.getState().me?.verification.verified === true
}

export const useGovernance = create<GovernanceState>()((set, get) => {
  const patchChange = (gen: number, patch: Partial<Change>): void => {
    const c = get().change
    if (gen === generation && c) set({ change: { ...c, ...patch } })
  }

  return {
    workspaces: null,
    teams: null,
    users: null,
    tokens: null,
    refused: {},
    change: null,

    load: (kind) => {
      const without = (): GovernanceState['refused'] =>
        Object.fromEntries(Object.entries(get().refused).filter(([k]) => k !== kind))
      set({ refused: without() })
      loadList(LIST_CMD[kind], (event) => listOf(kind, event), {
        loaded: (patch) => {
          set({ ...patch, refused: without() })
        },
        failed: (r) => {
          set({ [kind]: null, refused: { ...without(), [kind]: r } })
        },
      })
    },

    propose: (title, cmd, onResult) => {
      get().dismiss()
      const gen = ++generation
      resultHandler = onResult
      set({ change: { title, phase: 'asking', preview: null, refusal: null, created: null } })
      const id = sendForChange(cmd, gen, (event) => {
        if (event.type === 'Preview') patchChange(gen, { phase: 'preview', preview: event.data })
        else patchChange(gen, { phase: 'refused', refusal: refusalOf(event) })
      })
      if (id === null) patchChange(gen, { phase: 'refused', refusal: OFFLINE })
    },

    confirm: async () => {
      const gen = generation
      const c = get().change
      if (!c?.preview || c.phase !== 'preview') return
      const { preview } = c
      let stepUp: PasskeyAssertion | null | undefined
      if (preview.step_up && !recentlyVerified() && useSession.getState().me?.verification.enrolled) {
        patchChange(gen, { phase: 'stepping', refusal: null })
        stepUp = await usePasskeys.getState().requestStepUp(preview.summary)
        if (gen !== generation) return
        if (!stepUp) {
          patchChange(gen, { phase: 'preview' })
          return
        }
      }
      patchChange(gen, { phase: 'confirming', refusal: null })
      const cmd: Cmd = {
        type: 'Confirm',
        data: stepUp ? { preview_id: preview.preview_id, step_up: stepUp } : { preview_id: preview.preview_id },
      }
      const onResult = resultHandler
      const id = sendForChange(cmd, gen, (event) => {
        onResult?.(event)
        if (event.type === 'Confirmed') {
          patchChange(gen, { phase: 'done' })
          const s = get()
          for (const kind of loadedKinds(s)) s.load(kind)
        } else if (event.type === 'ApiTokenCreated') {
          patchChange(gen, { created: { token: event.data.token, secret: event.data.secret } })
        } else {
          const refusal = refusalOf(event)
          if (!refusal) return
          // A spent preview cannot be confirmed again: the person starts over.
          patchChange(gen, { phase: 'refused', refusal, preview: null })
          if (refusal === ENROL_FIRST) usePasskeys.getState().openEnrol(true)
        }
      })
      if (id === null) patchChange(gen, { phase: 'refused', refusal: OFFLINE })
    },

    dismiss: () => {
      generation++
      resultHandler = undefined
      for (const id of changeIds) replies.delete(id)
      changeIds = []
      set({ change: null })
    },
  }
})
