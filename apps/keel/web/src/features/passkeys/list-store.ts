/**
 * The Account → Passkeys page's state (spec 055): the person's own passkeys, and removing one.
 *
 * - `load` asks `ListPasskeys`; `Passkeys` is the list. A refresh keeps the old list on screen.
 *   The list reloads after any `PasskeyRegistered` (the enrol dialog, ./store.ts) while the page is
 *   open, takes a pushed `Passkeys`, and reloads after a reconnect.
 * - Removal is the two-step mutation: `startRemove` sends `PreviewPasskeyRemove` and shows the
 *   `Preview` for confirmation; `confirmRemove` asks for a passkey (`requestStepUp`) when the
 *   preview says `step_up` (the server always does; it skips the check itself while the window is
 *   fresh), then sends `Confirm` with it. The server answers `Confirmed` then `Passkeys` on the
 *   same `reply_to`; the second is the new list. `PasskeyRefused` (`last_passkey` and the rest),
 *   `PasskeyEnrolmentRequired`, `Denied` and `OpError` (`step_up_required`) show their message,
 *   through the passkey dialogs' mapping, and the list reloads.
 * - `add` opens the enrol dialog. A person who has passkeys must have checked it is them within
 *   the step-up window to register another (else `PasskeyBegin` answers `step_up_required`), so
 *   when the check is stale it first runs the step-up dialog and finishes that ceremony
 *   (`PasskeyFinish` → `Verification`).
 * - Removing a passkey moves the person's `authz_epoch`: the server keeps this session and revokes
 *   the person's other ones. Should this one get `Revoked` anyway, or stop being `ready`,
 *   everything here resets quietly and nothing more is sent: the shell signs out and shows the
 *   notice.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type {
  Cmd,
  Event,
  PasskeyAssertion,
  PasskeyInfo,
  Preview,
  RequestId,
  Verification,
} from '@/protocol/wire'

import { OFFLINE, refusalMessage, usePasskeys } from './store'

export type ListStatus = 'idle' | 'loading' | 'ready' | 'error'

/**
 * `previewing` waits for the `Preview`; `confirm` shows it; `verifying` waits on the step-up
 * dialog; `removing` waits for `Confirmed`; `refused` shows why it did not happen.
 */
export type RemovalPhase = 'previewing' | 'confirm' | 'verifying' | 'removing' | 'refused'

export interface Removal {
  passkey: PasskeyInfo
  phase: RemovalPhase
  preview: Preview | null
  error: string | null
}

export interface PasskeyListState {
  status: ListStatus
  items: PasskeyInfo[]
  /** Why the list could not load. */
  error: string | null
  removal: Removal | null
  /** Why `add` could not open the enrol dialog. */
  addError: string | null
  add: () => Promise<void>
  load: () => Promise<void>
  startRemove: (passkey: PasskeyInfo) => Promise<void>
  confirmRemove: () => Promise<void>
  cancelRemove: () => void
  /** Back to idle, dropping anything in flight. */
  reset: () => void
}

export const DROPPED = 'The connection dropped before Keel answered. Check the list, then try again.'

/** Ends a wait without a message: the session ended or the page reset. */
class Stopped extends Error {}

/** The session checked it is the person within the step-up window, by its own clock. */
export function freshCheck(v: Verification, now = Date.now()): boolean {
  if (!v.verified || v.at === null) return false
  const at = Date.parse(v.at)
  return Number.isFinite(at) && now < at + v.window_secs * 1000
}

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e))

const waiting = new Map<RequestId, { resolve: (e: Event) => void; reject: (e: Error) => void }>()

function failWaiting(err: Error): void {
  const all = [...waiting.values()]
  waiting.clear()
  for (const w of all) w.reject(err)
}

/** Send `cmd` and wait for the first event that answers it. */
function ask(cmd: Cmd): Promise<Event> {
  const id = sendCmd(cmd)
  if (id === null) return Promise.reject(new Error(OFFLINE))
  return new Promise((resolve, reject) => waiting.set(id, { resolve, reject }))
}

/** Bumped to drop a load or a removal in flight; each checks its own on every await. */
let loadGen = 0
let removalGen = 0
/** Bumped by `reset` only: the session ended or the page closed. */
let epoch = 0

export const usePasskeyList = create<PasskeyListState>()((set, get) => {
  const patch = (p: Partial<Removal>): void => {
    set((s) => (s.removal ? { removal: { ...s.removal, ...p } } : {}))
  }

  return {
    status: 'idle',
    items: [],
    error: null,
    removal: null,
    addError: null,

    add: async () => {
      set({ addError: null })
      const v = useSession.getState().me?.verification
      if (v?.enrolled && !freshCheck(v)) {
        const gen = epoch
        const assertion = await usePasskeys.getState().requestStepUp('Confirm it’s you before adding a passkey.')
        if (!assertion || gen !== epoch) return
        try {
          const event = await ask({ type: 'PasskeyFinish', data: assertion })
          if (gen !== epoch) return
          if (event.type !== 'Verification') {
            set({ addError: refusalMessage(event) })
            return
          }
        } catch (e) {
          if (gen === epoch && !(e instanceof Stopped)) set({ addError: message(e) })
          return
        }
      }
      usePasskeys.getState().openEnrol(false)
    },

    load: async () => {
      const gen = ++loadGen
      if (get().status !== 'ready') set({ status: 'loading', error: null })
      try {
        const event = await ask({ type: 'ListPasskeys' })
        if (gen !== loadGen) return
        if (event.type === 'Passkeys') set({ status: 'ready', items: event.data.items, error: null })
        else set({ status: 'error', error: refusalMessage(event) })
      } catch (e) {
        if (gen !== loadGen || e instanceof Stopped) return
        set({ status: 'error', error: message(e) })
      }
    },

    startRemove: async (passkey) => {
      const gen = ++removalGen
      set({ removal: { passkey, phase: 'previewing', preview: null, error: null } })
      try {
        const event = await ask({ type: 'PreviewPasskeyRemove', data: { id: passkey.id } })
        if (gen !== removalGen) return
        if (event.type === 'Preview') patch({ phase: 'confirm', preview: event.data })
        else patch({ phase: 'refused', error: refusalMessage(event) })
      } catch (e) {
        if (gen !== removalGen || e instanceof Stopped) return
        patch({ phase: 'refused', error: message(e) })
      }
    },

    confirmRemove: async () => {
      const r = get().removal
      if (r?.phase !== 'confirm' || !r.preview) return
      const gen = removalGen
      const { preview_id, step_up: needsStepUp, summary } = r.preview
      let stepUp: PasskeyAssertion | null = null
      if (needsStepUp) {
        patch({ phase: 'verifying', error: null })
        stepUp = await usePasskeys.getState().requestStepUp(summary)
        if (gen !== removalGen) return
        if (!stepUp) {
          patch({ phase: 'confirm' })
          return
        }
      }
      patch({ phase: 'removing', error: null })
      try {
        const event = await ask({
          type: 'Confirm',
          data: stepUp ? { preview_id, step_up: stepUp } : { preview_id },
        })
        if (gen !== removalGen) return
        if (event.type === 'Confirmed') {
          // `Passkeys`, the new list, follows on the same reply_to (handled below).
          removalGen++
          set({ removal: null })
          return
        }
        patch({ phase: 'refused', error: refusalMessage(event) })
        void get().load()
      } catch (e) {
        if (gen !== removalGen || e instanceof Stopped) return
        patch({ phase: 'refused', error: message(e) })
      }
    },

    cancelRemove: () => {
      removalGen++
      set({ removal: null })
    },

    reset: () => {
      const verifying = get().removal?.phase === 'verifying'
      epoch++
      loadGen++
      removalGen++
      failWaiting(new Stopped())
      set({ status: 'idle', items: [], error: null, removal: null, addError: null })
      // The step-up dialog the removal opened has nothing left to answer.
      if (verifying) usePasskeys.getState().close()
    },
  }
})

onServerEvent(({ reply_to, event }) => {
  if (reply_to !== undefined) {
    const w = waiting.get(reply_to)
    if (w) {
      waiting.delete(reply_to)
      w.resolve(event)
      return
    }
  }
  const list = usePasskeyList.getState()
  if (event.type === 'Revoked') {
    list.reset()
  } else if (list.status === 'idle') {
    // The page is not open: nothing to keep up to date.
  } else if (event.type === 'PasskeyRegistered') {
    void list.load()
  } else if (event.type === 'Passkeys') {
    // Pushed, or the list that follows `Confirmed` on a removal.
    usePasskeyList.setState({ status: 'ready', items: event.data.items, error: null })
  }
})

useSession.subscribe((now, before) => {
  if (before.status === 'ready' && now.status !== 'ready') {
    usePasskeyList.getState().reset()
  } else if (before.connection === 'open' && now.connection !== 'open') {
    // Whatever was in flight is lost with the socket (transport.ts).
    failWaiting(new Error(DROPPED))
  }
})

onReconnected(() => {
  const list = usePasskeyList.getState()
  if (list.status !== 'idle') void list.load()
})
