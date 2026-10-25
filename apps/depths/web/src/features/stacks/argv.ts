/**
 * Rendering the server's argv for people. The preview never builds a command: it shows the argv
 * the server returned, element for element. Joining it into one line only adds POSIX quoting where
 * an element would otherwise split or expand in a shell, so the copied line runs the same argv.
 */
import type { Actor, DenyReason, OperationKind, OpOutcome, PreviewVoidReason } from '@/protocol/generated'

import type { Refusal } from './ops-store'

const SAFE = /^[A-Za-z0-9_@%+=:,./-]+$/

export function quoteArg(arg: string): string {
  if (arg === '') return "''"
  if (SAFE.test(arg)) return arg
  return `'${arg.replaceAll("'", `'\\''`)}'`
}

export function argvLine(argv: readonly string[]): string {
  return argv.map(quoteArg).join(' ')
}

export const KIND_LABEL: Record<OperationKind, string> = {
  up: 'Up',
  stop: 'Stop',
  restart: 'Restart',
  down: 'Down',
  pull: 'Pull',
  update: 'Update',
  save: 'Save',
  create: 'Create',
  delete: 'Delete',
  service_up: 'Start service',
  service_stop: 'Stop service',
  service_restart: 'Restart service',
}

export const VOID_TEXT: Record<PreviewVoidReason, string> = {
  unknown: 'The server no longer knows this preview. Make a new one.',
  used: 'This preview was already used. Make a new one.',
  expired: 'This preview expired: previews last 5 minutes. Make a new one to see the command as it is now.',
  stack_changed: 'The stack changed since this preview was made, so it was not run. Make a new one to see the command as it is now.',
}

export const DENY_TEXT: Record<DenyReason, string> = {
  not_permitted: 'You are not permitted to do this.',
  pending: 'Your account has no access yet.',
  feature_disabled: 'This feature is turned off on this server.',
  owner: 'Owners cannot be changed here.',
  role_not_assignable: 'That role cannot be assigned.',
  untrusted_network:
    'Not from this network: it is not trusted yet. Stop, start, restart, logs and stats still work here.',
  shell_escape: 'This container can reach the host, so shells into it are refused.',
  shell_root_admin_only: 'This container runs as root on the host, so only an administrator may open a shell in it.',
  shell_unverifiable: 'The container could not be checked just now, so the shell was refused. Try again.',
  compose_blocked: 'This compose file lets a container reach the host. Only an administrator can deploy it, by acknowledging the findings.',
}

export function actorLabel(a: Actor): string {
  return `${a.username} (${a.provider})`
}

export function outcomeLabel(o: OpOutcome): string {
  switch (o.kind) {
    case 'succeeded':
      return 'Succeeded'
    case 'cancelled':
      return 'Cancelled'
    case 'failed':
      return o.exit_code === null ? 'Failed' : `Failed (exit ${String(o.exit_code)})`
  }
}

export function refusalText(r: Refusal): string {
  switch (r.kind) {
    case 'busy':
      return `${actorLabel(r.actor)} is already running ${r.running ? KIND_LABEL[r.running].toLowerCase() : 'an operation'} on ${r.stack} (operation ${r.operationId}). Wait for it to finish.`
    case 'denied':
      // The compose policy names the findings that blocked it (051 FR-018).
      return r.reason === 'compose_blocked' && r.detail ? `${DENY_TEXT[r.reason]} ${r.detail}` : DENY_TEXT[r.reason]
    case 'invalid':
      return r.reason
    case 'void':
      return VOID_TEXT[r.reason]
    case 'validation':
      return `The file does not validate: ${r.errors.map((e) => (e.line === null ? e.message : `line ${String(e.line)}: ${e.message}`)).join('; ')}`
    case 'conflict':
      return 'The files changed on disk since you opened them. Nothing was written.'
    case 'lost':
      return 'The connection dropped before the server answered. Check the stack before trying again.'
  }
}
