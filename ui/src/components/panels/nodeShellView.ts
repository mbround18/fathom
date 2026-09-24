/** Pure view-model helpers for the Node Shell panel (037-node-shell), unit-tested in Vitest. */
import type { NodeShellAccessCheck, NodeShellPhase } from '@/protocol/types'

export interface PhaseView {
  label: string
  /** A start/cleanup is in flight: show progress, disable Start. */
  busy: boolean
  /** The terminal should be mounted. */
  showTerminal: boolean
  /** Cancel/Stop is meaningful (a pod exists or is being created). */
  canStop: boolean
  tone: 'neutral' | 'ok' | 'warn' | 'error'
}

export function phaseView(phase: NodeShellPhase, leftover: boolean): PhaseView {
  switch (phase) {
    case 'Creating':
      return {
        label: 'Creating debug pod…',
        busy: true,
        showTerminal: false,
        canStop: true,
        tone: 'neutral',
      }
    case 'Pending':
      return {
        label: 'Waiting for the pod to start…',
        busy: true,
        showTerminal: false,
        canStop: true,
        tone: 'neutral',
      }
    case 'Running':
    case 'Attached':
      return {
        label: phase === 'Attached' ? 'Shell attached' : 'Pod running, attaching…',
        busy: false,
        showTerminal: true,
        canStop: true,
        tone: 'ok',
      }
    case 'Terminating':
      return {
        label: 'Deleting debug pod…',
        busy: true,
        showTerminal: false,
        canStop: false,
        tone: 'warn',
      }
    case 'Removed':
      return {
        label: 'Debug pod removed',
        busy: false,
        showTerminal: false,
        canStop: false,
        tone: 'neutral',
      }
    case 'Failed':
      // A leftover pod still exists: offer Stop again so the delete can be retried.
      return {
        label: leftover ? 'Debug pod could not be deleted' : 'Node shell failed',
        busy: false,
        showTerminal: false,
        canStop: leftover,
        tone: 'error',
      }
  }
}

export function summarizeAccess(checks: readonly NodeShellAccessCheck[]): {
  allowed: boolean
  problems: string[]
} {
  const problems: string[] = []
  for (const c of checks) {
    if (c.outcome.type === 'Allowed') continue
    const what = `${c.verb} ${c.resource}${c.subresource ? `/${c.subresource}` : ''}`
    const why =
      c.outcome.type === 'Denied'
        ? (c.outcome.data.reason ?? 'denied')
        : `could not be checked: ${c.outcome.data.message}`
    problems.push(`${what}: ${why}`)
  }
  // Fail safe: no answer yet is not permission.
  return { allowed: checks.length > 0 && problems.length === 0, problems }
}

/** The debug pod image is Linux-only; the reason shown when the target node is Windows. */
export const WINDOWS_REASON = 'Node Shell supports Linux nodes only; this node runs Windows.'

export function isWindowsNode(os: string | undefined): boolean {
  return os?.toLowerCase() === 'windows'
}
