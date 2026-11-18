/**
 * The converge commands and events (spec 068 contracts/protocol.md), in one place.
 *
 * The variants are the generated `Cmd`/`Event` unions narrowed to these names, so a rename in
 * keel-protocol fails the typecheck here.
 *
 * Type imports only, so the Playwright fake (tests/support/fake-converge.ts) shares these names.
 */
import type { Cmd, Event } from '../../protocol/wire'

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

type CmdName = (typeof CMD)[keyof typeof CMD]
type EventName = (typeof EVENT)[keyof typeof EVENT]

export type ConvergeCmd = Extract<Cmd, { type: CmdName }>
export type ConvergeEvent = Extract<Event, { type: EventName }>

const NAMES: ReadonlySet<string> = new Set(Object.values(EVENT))

/** The converge command as a generated `Cmd`, to send on the shared socket. */
export function asCmd(cmd: ConvergeCmd): Cmd {
  return cmd
}

/** The event as a converge event, or `null` when it is not one. */
export function convergeEvent(event: Event): ConvergeEvent | null {
  return NAMES.has(event.type) ? (event as ConvergeEvent) : null
}
