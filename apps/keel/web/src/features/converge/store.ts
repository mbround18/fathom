/**
 * Converge (spec 068 US1, T026/T027): the preview of a converge and the live view of one.
 *
 * Preview: `PreviewConverge` (or `PreviewConvergeRetry` for Retry failed) answers a `ConvergePlan`
 * with a preview token; Confirm sends the token back and the server answers the new parent run,
 * which the page opens. Converge confirms carry no passkey step-up (contracts/protocol.md): in
 * signed mode each step's commit is reviewed on the review page, one passkey assertion per step.
 *
 * View: `GetConverge` answers a `ConvergeView`. It is re-read when the server says something moved:
 * `ConvergeChanged` for the run, or `RunState` for the parent or any of its child runs.
 *
 * Every command and event name is in ./wire.ts. `OpError`, `Denied` and the converge replies count
 * only when they answer a command sent from here.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent } from '@/app/connection'
import { navigate } from '@companion/ui/kit'
import type { ConvergePlan, ConvergeTarget, ConvergeView, RequestId } from '@/protocol/wire'

import { runIds } from './derive'
import { CMD, convergeEvent, EVENT, sendConverge, type ConvergeCmd } from './wire'

const OFFLINE = 'Keel is not connected. Wait for it to reconnect, then try again.'

/** The OpError code the server answers with while `KEEL_CONVERGE` is off. */
export const FEATURE_DISABLED = 'feature_disabled'

/** What the preview was asked for, so the Check toggle can ask again. */
export type Asked = { kind: 'target'; target: ConvergeTarget } | { kind: 'retry'; run_id: string; replan: boolean }

export interface ConvergeState {
  /** The preview open, or `null`. */
  asked: Asked | null
  plan: ConvergePlan | null
  token: string | null
  /** A Confirm is on its way. */
  confirming: boolean
  error: string | null
  /** The server said converge is off (`KEEL_CONVERGE`); the entry points hide. */
  disabled: boolean
  /** Converge views by parent run id. */
  views: Record<string, ConvergeView>
  /** The run whose view is open on the converge page. */
  watching: string | null

  preview: (target: ConvergeTarget) => void
  /** Retry failed on a finished converge; `replan` plans again at the binding head. */
  retry: (run_id: string, replan: boolean) => void
  /** Turns check mode on or off and asks for the plan again. */
  setCheck: (check: boolean) => void
  confirm: () => void
  close: () => void
  /** Reads `run_id`'s view (for the host panel's Last converge). */
  fetch: (run_id: string) => void
  watch: (run_id: string | null) => void
  cancelRun: (run_id: string) => void
  clearError: () => void
}

const mine = new Set<RequestId>()

function send(cmd: ConvergeCmd): boolean {
  const id = sendConverge(cmd)
  if (id === null) {
    useConverge.setState({ error: OFFLINE, confirming: false })
    return false
  }
  mine.add(id)
  return true
}

function ask(asked: Asked): void {
  useConverge.setState({ asked, plan: null, token: null, error: null, confirming: false })
  if (asked.kind === 'target') send({ type: CMD.preview, data: asked.target })
  else send({ type: CMD.previewRetry, data: { run_id: asked.run_id, replan: asked.replan } })
}

export const useConverge = create<ConvergeState>()((set, get) => ({
  asked: null,
  plan: null,
  token: null,
  confirming: false,
  error: null,
  disabled: false,
  views: {},
  watching: null,

  preview: (target) => {
    ask({ kind: 'target', target })
  },
  retry: (run_id, replan) => {
    ask({ kind: 'retry', run_id, replan })
  },
  setCheck: (check) => {
    const asked = get().asked
    if (asked?.kind !== 'target' || asked.target.check === check) return
    ask({ kind: 'target', target: { ...asked.target, check } })
  },
  confirm: () => {
    const { token, asked, confirming } = get()
    if (!token || !asked || confirming) return
    set({ confirming: true, error: null })
    send({ type: asked.kind === 'retry' ? CMD.confirmRetry : CMD.confirm, data: { preview_token: token } })
  },
  close: () => {
    set({ asked: null, plan: null, token: null, confirming: false, error: null })
  },
  fetch: (run_id) => {
    send({ type: CMD.get, data: { run_id } })
  },
  watch: (run_id) => {
    set({ watching: run_id })
    if (run_id !== null) send({ type: CMD.get, data: { run_id } })
  },
  cancelRun: (run_id) => {
    set({ error: null })
    send({ type: CMD.cancel, data: { run_id } })
  },
  clearError: () => {
    set({ error: null })
  },
}))

/** The open view names `run` (the parent or one of its steps). */
function touches(run: string): boolean {
  const { watching, views } = useConverge.getState()
  if (watching === null) return false
  const view = views[watching]
  return run === watching || (view !== undefined && runIds(view).has(run))
}

function refresh(): void {
  const { watching } = useConverge.getState()
  if (watching !== null) send({ type: CMD.get, data: { run_id: watching } })
}

onServerEvent(({ reply_to, event }) => {
  const ours = reply_to !== undefined && mine.has(reply_to)
  const c = convergeEvent(event)
  if (c) {
    switch (c.type) {
      case EVENT.plan:
        if (ours && useConverge.getState().asked)
          useConverge.setState({ plan: c.data.plan, token: c.data.preview_token, error: null, disabled: false })
        break
      case EVENT.started:
        if (ours) {
          useConverge.getState().close()
          navigate(`/runs/${c.data.run_id}`)
        }
        break
      case EVENT.view: {
        const view = c.data
        useConverge.setState((s) => ({ views: { ...s.views, [view.run.id]: view }, disabled: false }))
        break
      }
      case EVENT.changed:
        if (c.data.run_id === useConverge.getState().watching) refresh()
        break
    }
    return
  }
  switch (event.type) {
    case 'RunState':
      if (touches(event.data.run)) refresh()
      break
    case 'OpError':
      if (ours) {
        if (event.data.code === FEATURE_DISABLED) useConverge.setState({ disabled: true, asked: null, plan: null })
        useConverge.setState({ error: event.data.message, confirming: false })
      }
      break
    case 'Denied':
      if (ours) useConverge.setState({ error: `Not allowed: ${event.data.reason}`, confirming: false })
      break
    default:
      break
  }
})

onReconnected(() => {
  const { asked, watching } = useConverge.getState()
  if (watching !== null) refresh()
  // A preview token does not outlive the connection: ask again.
  if (asked) ask(asked)
})
