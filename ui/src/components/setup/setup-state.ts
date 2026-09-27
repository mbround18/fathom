/**
 * 043 setup screen: which app-wide setup state, if any, replaces the navigator, the slot area
 * and the dock (data-model.md § SetupState). Pure, so every transition is unit tested.
 *
 * Built on `firstRunOf`: the "no usable kubeconfig" and "nothing selected" cases are the same
 * decisions the per-pane first-run state makes. A connection problem is a setup state only for
 * a context picked on the setup screen; otherwise it stays the per-pane `connection` state.
 */
import { firstRunOf } from '@/lib/first-run'
import type { ConnectionProblem, KubeconfigIssue } from '@/protocol/types'

export type SetupState =
  /** Missing, empty, malformed or unreadable kubeconfig. */
  | { kind: 'kubeconfig'; issue: KubeconfigIssue; paths: string[] }
  /** Contexts exist but none is selected. */
  | { kind: 'pick'; contexts: string[]; paths: string[] }
  /** A context picked in setup is being selected and connected. */
  | { kind: 'connecting'; context: string }
  /** A context picked in setup could not connect. */
  | { kind: 'failed'; context: string; problem: ConnectionProblem }
  /** The optional sign-in step after a pick. */
  | { kind: 'sign-in'; context: string }

export interface SetupKubeconfig {
  /** `ContextsUpdated` / `KubeconfigStatus` has arrived. */
  loaded: boolean
  contexts: readonly string[]
  issue?: KubeconfigIssue
  /** The kubeconfig files read (`KubeconfigStatus.paths`). */
  paths: readonly string[]
}

export interface SetupConnection {
  /** The active context's API server has answered. */
  connected: boolean
  /** The active context's classified connection failure. */
  problem?: ConnectionProblem
}

export interface SetupInputs {
  kubeconfig: SetupKubeconfig
  activeContext?: string
  /** Connection state of `activeContext`. */
  connection?: SetupConnection
  /** Hosted mode never shows the setup screen. */
  hosted: boolean
  /** The context picked on the setup screen, until setup finishes or the operator goes back. */
  pickedInSetup?: string
  /** The sign-in step is offered and has not been skipped or finished. */
  signInStep?: boolean
}

/** `undefined` when the normal shell should render. */
export function setupStateOf(i: SetupInputs): SetupState | undefined {
  if (i.hosted) return undefined
  const kc = i.kubeconfig
  const firstRun = firstRunOf({
    contextsLoaded: kc.loaded,
    contextCount: kc.contexts.length,
    activeContext: i.activeContext,
    issue: kc.issue,
    problem: i.connection?.problem,
  })
  if (firstRun?.kind === 'kubeconfig') {
    const paths = firstRun.issue.paths.length > 0 ? firstRun.issue.paths : kc.paths
    return { kind: 'kubeconfig', issue: firstRun.issue, paths: [...paths] }
  }
  if (!kc.loaded) return undefined

  const picked = i.pickedInSetup
  // A pick counts only while the context still exists and nothing else was selected since.
  if (
    picked !== undefined &&
    kc.contexts.includes(picked) &&
    (i.activeContext === undefined || i.activeContext === picked)
  ) {
    if (i.signInStep) return { kind: 'sign-in', context: picked }
    if (i.activeContext === picked) {
      const problem = i.connection?.problem
      if (problem) return { kind: 'failed', context: picked, problem }
      if (i.connection?.connected) return undefined
    }
    return { kind: 'connecting', context: picked }
  }

  if (firstRun?.kind === 'pick') {
    return { kind: 'pick', contexts: [...kc.contexts], paths: [...kc.paths] }
  }
  // A connection problem outside setup stays the per-pane first-run state.
  return undefined
}
