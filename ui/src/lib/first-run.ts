/**
 * First-run and connection-failure UX: which explanation a pane shows instead of its content
 * when there is nothing (usable) to connect to, and the copy for it.
 *
 * The backend classifies the problem (`KubeconfigStatus`, `ActiveConnectionProblem`); this
 * module only turns that into a title and a next step. The verbatim error stays available in
 * the details disclosure (Principle I), it is never the headline.
 */
import type { ConnectionProblem, KubeconfigIssue } from '@/protocol/types'

export type FirstRun =
  /** No usable kubeconfig: missing, empty, malformed or unreadable. */
  | { kind: 'kubeconfig'; issue: KubeconfigIssue }
  /** Contexts exist but none is selected (or `current-context` names one that is not defined). */
  | { kind: 'pick'; issue?: KubeconfigIssue }
  /** The active context is selected but its cluster cannot be used. */
  | { kind: 'connection'; context: string; problem: ConnectionProblem }

export interface FirstRunInputs {
  contextsLoaded: boolean
  contextCount: number
  activeContext?: string | undefined
  issue?: KubeconfigIssue | undefined
  problem?: ConnectionProblem | undefined
}

/** `undefined` when the pane should render normally. */
export function firstRunOf(i: FirstRunInputs): FirstRun | undefined {
  if (!i.contextsLoaded) return undefined
  if (i.issue && i.issue.kind !== 'NoCurrentContext') return { kind: 'kubeconfig', issue: i.issue }
  if (i.contextCount === 0) {
    return { kind: 'kubeconfig', issue: i.issue ?? { kind: 'Empty', paths: [], detail: '' } }
  }
  if (i.activeContext === undefined)
    return i.issue ? { kind: 'pick', issue: i.issue } : { kind: 'pick' }
  if (i.problem) return { kind: 'connection', context: i.activeContext, problem: i.problem }
  return undefined
}

export interface Copy {
  title: string
  detail: string
}

function pathList(paths: readonly string[]): string {
  return paths.length === 0 ? 'your kubeconfig' : paths.join(', ')
}

export function kubeconfigIssueCopy(issue: KubeconfigIssue): Copy {
  const where = pathList(issue.paths)
  switch (issue.kind) {
    case 'Missing':
      return {
        title: 'No kubeconfig found',
        detail: `Fathom looked for ${where} and found nothing. Create a local kind cluster to get started, or point KUBECONFIG at an existing kubeconfig and reload.`,
      }
    case 'Empty':
      return {
        title: 'No clusters configured',
        detail: `${issue.paths.length === 0 ? 'The kubeconfig' : where} defines no contexts. Create a local kind cluster, or add a context (for example with your cloud CLI) and reload.`,
      }
    case 'Malformed':
      return {
        title: 'Kubeconfig could not be parsed',
        detail: `${where} is not valid kubeconfig YAML${issue.detail ? `: ${issue.detail}` : ''}. Fix the file and reload.`,
      }
    case 'Unreadable':
      return {
        title: 'Kubeconfig could not be opened',
        detail: `${where}${issue.detail ? `: ${issue.detail}` : ''}. Check the path and its permissions, then reload.`,
      }
    case 'NoCurrentContext':
      return pickCopy(issue)
  }
}

export function pickCopy(issue?: KubeconfigIssue): Copy {
  return {
    title: 'Pick a context',
    detail: issue?.detail
      ? `${issue.detail} Choose the cluster to open.`
      : 'Your kubeconfig does not select a current context. Choose the cluster to open.',
  }
}

export function connectionProblemCopy(context: string, problem: ConnectionProblem): Copy {
  const at = problem.server ?? 'the API server'
  switch (problem.kind) {
    case 'Unreachable':
      return {
        title: `Cannot reach ${context}`,
        detail: `Nothing answered at ${at}. The cluster may be stopped or deleted. Start it and retry, pick another context, or create a local kind cluster.`,
      }
    case 'Dns':
      return {
        title: `Cannot resolve ${context}`,
        detail: `The host name in ${at} did not resolve. Check your network or VPN, then retry.`,
      }
    case 'Timeout':
      return {
        title: `${context} timed out`,
        detail: `${at} did not answer in time. Check your network or VPN, then retry.`,
      }
    case 'Tls':
      return {
        title: `Cannot verify ${context}`,
        detail: `The TLS handshake with ${at} failed. The certificate data in the kubeconfig may be stale or the server changed; refresh it and retry.`,
      }
    case 'Unauthorized':
      return {
        title: `${context} rejected the credentials`,
        detail: `${at} answered 401 Unauthorized: the token or client certificate has expired or is invalid. Refresh the credentials (for example log in again with your cloud CLI) and retry.`,
      }
    case 'Forbidden':
      return {
        title: `${context} denied access`,
        detail: `The credentials were accepted but may not read the API server (403 Forbidden). Ask for access, or pick another context.`,
      }
    case 'Other':
      return {
        title: `Cannot connect to ${context}`,
        detail:
          'The connection failed. The details below have the exact error. Retry, or pick another context.',
      }
  }
}

export interface ReloadResult {
  issue?: KubeconfigIssue | undefined
  /** `KubeconfigStatus.paths`: the files read, or the paths the issue is about. */
  paths: readonly string[]
  contextCount: number
}

/**
 * The one-line outcome of "Reload kubeconfig" (feedback #1): shown even when nothing changed,
 * so the button never looks dead.
 */
export function reloadOutcome(r: ReloadResult): string {
  const where = pathList(r.paths.length > 0 ? r.paths : (r.issue?.paths ?? []))
  switch (r.issue?.kind) {
    case 'Missing':
      return `No kubeconfig found; looked in ${where}.`
    case 'Empty':
      return `Reloaded: ${where} defines no contexts.`
    case 'Malformed':
      return `Reloaded: ${where} could not be parsed.`
    case 'Unreadable':
      return `Reloaded: ${where} could not be opened.`
    case 'NoCurrentContext':
    case undefined: {
      const n = r.contextCount
      return `Found ${n} context${n === 1 ? '' : 's'} in ${where}.`
    }
  }
}

/** One line for a pane that has no room for the full state (a list's error strip). */
export function connectionProblemSummary(context: string, problem: ConnectionProblem): string {
  return connectionProblemCopy(context, problem).title
}
