/**
 * A host's profiles (spec 068 US3, T040): the editor and the inventory's suggestions.
 *
 * Editing is two-step, like every change. `PreviewHostProfiles` answers `HostProfilesPreview`
 * (what is added and removed, profiles outside the host's ceiling, the certificate principals
 * after, the converge steps after) and then the generic `Preview`; `Confirm` always needs a passkey
 * unless the session has just verified. The answer is `Confirmed` and `Host`; the server pushes
 * `HostProfilesChanged` to the workspace.
 *
 * Suggestions come from the repository's `groups:` map (`ListProfileSuggestions`, answered by
 * `ProfileSuggestions`). Accept opens the editor's preview with the suggestion applied; Dismiss
 * (`DismissProfileSuggestion`) answers the host's remaining suggestions. The server pushes
 * `ProfileSuggestionsChanged` after a sync that moved some.
 *
 * Replies count only when they answer a command sent from here, and only for the flow that sent
 * them: closing or going back drops a late answer.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { OFFLINE, refusalOf, type Refusal } from '@/app/load'
import { useSession } from '@/app/session-store'
import { usePasskeys } from '@/features/passkeys/store'
import type {
  Cmd,
  Event,
  HostInfo,
  HostProfilesPreview,
  PasskeyAssertion,
  Preview,
  ProfileSuggestion,
  RequestId,
} from '@/protocol/wire'

import { applySuggestion, changeOf } from './profiles'
import { useHosts } from './store'

export type ProfilesPhase = 'closed' | 'edit' | 'asking' | 'preview' | 'stepping' | 'confirming' | 'refused'

const ENROL_FIRST: Refusal = {
  kind: 'error',
  code: 'passkey_required',
  message: 'This change needs a passkey. Register one, then confirm again.',
}

/** The OpError code while `KEEL_CONVERGE` is off. */
const FEATURE_DISABLED = 'feature_disabled'

export interface ProfilesState {
  /** The host being edited. */
  host: HostInfo | null
  /** Its profiles as the editor has them. */
  draft: string[]
  phase: ProfilesPhase
  /** What the change does, and the `Preview` that Confirm answers. */
  preview: HostProfilesPreview | null
  pending: Preview | null
  refusal: Refusal | null
  /** Open suggestions by host id, once read. */
  suggestions: Record<string, ProfileSuggestion[]>
  /** Why the suggestions could not be read, by host id. */
  suggestionsFailed: Record<string, Refusal>
  /** The suggestion a Dismiss is on its way for (`host/profile/action`). */
  dismissing: string | null
  /** The server answered `feature_disabled`: suggestions and editing hide. */
  disabled: boolean

  /** Opens the editor on `host`'s profiles. */
  edit: (host: HostInfo) => void
  /** Accept: the editor's preview with `s` applied. */
  accept: (host: HostInfo, s: ProfileSuggestion) => void
  setDraft: (next: string[]) => void
  /** Sends the draft for a preview. */
  review: () => void
  confirm: () => Promise<void>
  /** Back to the editor, from a preview or a refusal. */
  back: () => void
  close: () => void
  loadSuggestions: (host: HostInfo) => void
  dismiss: (s: ProfileSuggestion) => void
}

/** A suggestion's key, for `dismissing`. */
export function suggestionKey(s: Pick<ProfileSuggestion, 'host_id' | 'profile' | 'action'>): string {
  return `${s.host_id}/${s.profile}/${s.action}`
}

const handlers = new Map<RequestId, (event: Event) => void>()
let generation = 0

onServerEvent(({ reply_to, event }) => {
  if (reply_to !== undefined) handlers.get(reply_to)?.(event)
})

/** Sends `cmd`; `handle` gets its answers, unless `flow` is set and the flow moved on. */
function send(cmd: Cmd, handle: (event: Event) => void, flow = true): boolean {
  const gen = generation
  const id = sendCmd(cmd)
  if (id === null) return false
  handlers.set(id, (event) => {
    if (!flow || gen === generation) handle(event)
  })
  return true
}

function refusal(event: Event): Refusal | null {
  if (event.type === 'PasskeyEnrolmentRequired') return ENROL_FIRST
  return refusalOf(event)
}

export const useProfiles = create<ProfilesState>()((set, get) => {
  const setSuggestions = (host: string, items: ProfileSuggestion[]): void => {
    set((s) => {
      const failed = Object.fromEntries(Object.entries(s.suggestionsFailed).filter(([id]) => id !== host))
      return { suggestions: { ...s.suggestions, [host]: items.filter((i) => i.host_id === host) }, suggestionsFailed: failed }
    })
    // The Hosts list's dot follows the count.
    const n = items.filter((i) => i.host_id === host).length
    useHosts.setState((h) => ({ hosts: h.hosts.map((x) => (x.id === host && x.suggestions !== n ? { ...x, suggestions: n } : x)) }))
  }

  return {
    host: null,
    draft: [],
    phase: 'closed',
    preview: null,
    pending: null,
    refusal: null,
    suggestions: {},
    suggestionsFailed: {},
    dismissing: null,
    disabled: false,

    edit: (host) => {
      generation++
      set({ host, draft: [...host.profiles], phase: 'edit', preview: null, pending: null, refusal: null })
    },

    accept: (host, s) => {
      get().edit(host)
      set({ draft: applySuggestion(host.profiles, s) })
      get().review()
    },

    setDraft: (draft) => {
      set({ draft })
    },

    review: () => {
      const { host, draft } = get()
      if (!host) return
      generation++
      const { add, remove } = changeOf(host.profiles, draft)
      set({ phase: 'asking', preview: null, pending: null, refusal: null })
      const sent = send({ type: 'PreviewHostProfiles', data: { host_id: host.id, add, remove } }, (event) => {
        if (event.type === 'HostProfilesPreview') set({ preview: event.data.preview })
        else if (event.type === 'Preview') set({ phase: 'preview', pending: event.data })
        else {
          const r = refusal(event)
          if (r) set({ phase: 'refused', refusal: r })
        }
      })
      if (!sent) set({ phase: 'refused', refusal: OFFLINE })
    },

    confirm: async () => {
      const { pending, phase, host } = get()
      if (!pending || !host || phase !== 'preview') return
      const gen = generation
      const me = useSession.getState().me
      let stepUp: PasskeyAssertion | null = null
      if (pending.step_up && !me?.verification.verified && me?.verification.enrolled) {
        set({ phase: 'stepping' })
        stepUp = await usePasskeys.getState().requestStepUp(pending.summary)
        if (gen !== generation) return
        if (!stepUp) {
          set({ phase: 'preview' })
          return
        }
      }
      set({ phase: 'confirming', refusal: null })
      const sent = send(
        {
          type: 'Confirm',
          data: stepUp ? { preview_id: pending.preview_id, step_up: stepUp } : { preview_id: pending.preview_id },
        },
        (event) => {
          if (event.type === 'Confirmed') {
            get().close()
            // The change settles suggestions; read what is left.
            get().loadSuggestions(host)
            return
          }
          const r = refusal(event)
          if (!r) return
          // A spent preview cannot be confirmed again: the person starts over from the editor.
          set({ phase: 'refused', refusal: r, pending: null })
          if (r === ENROL_FIRST) usePasskeys.getState().openEnrol(true)
        },
      )
      if (!sent) set({ phase: 'refused', refusal: OFFLINE, pending: null })
    },

    back: () => {
      generation++
      set({ phase: 'edit', preview: null, pending: null, refusal: null })
    },

    close: () => {
      generation++
      set({ host: null, draft: [], phase: 'closed', preview: null, pending: null, refusal: null })
    },

    loadSuggestions: (host) => {
      const sent = send(
        { type: 'ListProfileSuggestions', data: { workspace: host.workspace, host_id: host.id } },
        (event) => {
          if (event.type === 'ProfileSuggestions') {
            set({ disabled: false })
            setSuggestions(host.id, event.data.items)
            return
          }
          const r = refusalOf(event)
          if (!r) return
          if (r.kind === 'error' && r.code === FEATURE_DISABLED) set({ disabled: true })
          else set((s) => ({ suggestionsFailed: { ...s.suggestionsFailed, [host.id]: r } }))
        },
        false,
      )
      if (!sent) set((s) => ({ suggestionsFailed: { ...s.suggestionsFailed, [host.id]: OFFLINE } }))
    },

    dismiss: (s) => {
      const key = suggestionKey(s)
      set({ dismissing: key })
      const sent = send(
        { type: 'DismissProfileSuggestion', data: { host_id: s.host_id, profile: s.profile, action: s.action } },
        (event) => {
          if (event.type === 'ProfileSuggestions') {
            setSuggestions(s.host_id, event.data.items)
            if (get().dismissing === key) set({ dismissing: null })
            return
          }
          const r = refusalOf(event)
          if (!r) return
          set((st) => ({
            dismissing: st.dismissing === key ? null : st.dismissing,
            suggestionsFailed: { ...st.suggestionsFailed, [s.host_id]: r },
          }))
        },
        false,
      )
      if (!sent) set((st) => ({ dismissing: null, suggestionsFailed: { ...st.suggestionsFailed, [s.host_id]: OFFLINE } }))
    },
  }
})

/** The open host's detail, if it is `host`. */
function openHost(host: string): HostInfo | null {
  const { open, detail } = useHosts.getState()
  return open === host && detail?.host.id === host ? detail.host : null
}

onServerEvent(({ event }) => {
  switch (event.type) {
    case 'HostProfilesChanged': {
      const { host_id, profiles } = event.data
      useHosts.setState((h) => ({
        hosts: h.hosts.map((x) => (x.id === host_id ? { ...x, profiles } : x)),
        detail: h.detail?.host.id === host_id ? { ...h.detail, host: { ...h.detail.host, profiles } } : h.detail,
      }))
      break
    }
    case 'ProfileSuggestionsChanged': {
      const { hosts } = event.data
      const { workspace, hosts: listed, load } = useHosts.getState()
      // The list's counts moved: re-read it, and the open host's suggestions.
      if (workspace && listed.some((h) => hosts.includes(h.id))) load(workspace)
      for (const id of hosts) {
        const host = openHost(id)
        if (host) useProfiles.getState().loadSuggestions(host)
      }
      break
    }
    default:
      break
  }
})

onReconnected(() => {
  const { phase, back } = useProfiles.getState()
  // A preview does not outlive the connection: back to the editor, which asks again.
  if (phase === 'asking' || phase === 'preview' || phase === 'confirming') back()
})
