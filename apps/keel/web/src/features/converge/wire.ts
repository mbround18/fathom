/**
 * The converge commands and events (spec 068 contracts/protocol.md), in one place.
 *
 * The payload types are the generated bindings (`ConvergeTarget`, `ConvergePlan`, `ConvergeView`,
 * ...). The `Cmd`/`Event` variants that carry them are not in the generated unions yet, so their
 * names and shapes are declared here, and `asCmd`/`convergeEvent` are the only two places that
 * cross from these to the generated `Cmd`/`Event`. Once keel-protocol has the variants, align the
 * names in `CMD` and `EVENT` and drop the casts; nothing else in the app names them.
 *
 * Type imports only, so the Playwright fake (tests/support/fake-converge.ts) shares these names.
 */
import type { Cmd, ConvergeHostState, ConvergePlan, ConvergeTarget, ConvergeView, Event } from '../../protocol/wire'

/** Command names (`Cmd.type`). */
export const CMD = {
  preview: 'PreviewConverge',
  confirm: 'ConfirmConverge',
  previewRetry: 'PreviewConvergeRetry',
  confirmRetry: 'ConfirmConvergeRetry',
  cancel: 'CancelConverge',
  get: 'GetConverge',
} as const

/** Event names (`Event.type`). */
export const EVENT = {
  /** Answers `PreviewConverge` and `PreviewConvergeRetry`. */
  plan: 'ConvergePlan',
  /** Answers `ConfirmConverge` and `ConfirmConvergeRetry`: the new parent run. */
  started: 'ConvergeStarted',
  /** Answers `GetConverge`. */
  view: 'Converge',
  /** Pushed on a host-level change (holding, finished). */
  changed: 'ConvergeChanged',
} as const

export type ConvergeCmd =
  | { type: typeof CMD.preview; data: ConvergeTarget }
  | { type: typeof CMD.confirm; data: { preview_token: string } }
  | { type: typeof CMD.previewRetry; data: { run_id: string; replan: boolean } }
  | { type: typeof CMD.confirmRetry; data: { preview_token: string } }
  | { type: typeof CMD.cancel; data: { run_id: string } }
  | { type: typeof CMD.get; data: { run_id: string } }

export type ConvergeEvent =
  | { type: typeof EVENT.plan; data: { preview_token: string; plan: ConvergePlan } }
  | { type: typeof EVENT.started; data: { run_id: string } }
  | { type: typeof EVENT.view; data: ConvergeView }
  | { type: typeof EVENT.changed; data: { run_id: string; host_id: string; state: ConvergeHostState } }

const NAMES: ReadonlySet<string> = new Set(Object.values(EVENT))

/** The converge command as a generated `Cmd`, to send on the shared socket. */
export function asCmd(cmd: ConvergeCmd): Cmd {
  return cmd as unknown as Cmd
}

/** The event as a converge event, or `null` when it is not one. */
export function convergeEvent(event: Event): ConvergeEvent | null {
  const type = (event as { type: string }).type
  return NAMES.has(type) ? (event as unknown as ConvergeEvent) : null
}
