/**
 * 043 setup screen: the transient setup flow (never persisted) and the pure helpers the store
 * uses to decide what the setup screen shows and which context to open at startup.
 */
import type { SetupInputs } from '@/components/setup/setup-state'
import type { SectionKey } from '@/protocol/types'

import type { StoreData } from './index'

export interface SetupFlow {
  /** The context picked on the setup screen, until setup finishes. */
  picked?: string | undefined
  /** The optional sign-in step is showing. */
  signInStep?: boolean | undefined
  /** The sign-in step was skipped or finished for this pick (a 401 no longer re-offers it). */
  signInDone?: boolean | undefined
  /** "Choose another context" after a failed connect: the context list is back. */
  choosing?: boolean | undefined
  /**
   * A page opened while the setup screen shows (for example from the header Settings menu).
   * A clusterless page replaces the setup screen with "Back to setup"; any other section
   * leaves the setup screen in place.
   */
  detour?: { paneId: string; section: SectionKey } | undefined
  /** The remembered context was considered once both the UI config and contexts arrived. */
  startupTried?: boolean | undefined
}

export const INITIAL_SETUP: SetupFlow = {}

/** The store slices the setup state is derived from. */
export type SetupSource = Pick<StoreData, 'session' | 'hosted' | 'setup'>

/** Whether the active context's API server has answered. */
function isConnected(s: SetupSource, ctx: string): boolean {
  if (s.session.contextStatuses[ctx]?.status === 'Connected') return true
  // A namespace list for the new context also proves the API server answered (the switch
  // clears `namespaces`, so a non-empty list belongs to this context).
  return s.session.namespaces.length > 0 && s.session.connectionProblems[ctx] === undefined
}

/** `setupStateOf`'s inputs, read from the store. */
export function setupInputsOf(s: SetupSource): SetupInputs {
  const ctx = s.session.activeContext
  const problem = ctx === undefined ? undefined : s.session.connectionProblems[ctx]
  return {
    kubeconfig: {
      loaded: s.session.contextsLoaded,
      contexts: s.session.contexts,
      paths: s.session.kubeconfigPaths,
      ...(s.session.kubeconfigIssue ? { issue: s.session.kubeconfigIssue } : {}),
    },
    ...(ctx !== undefined
      ? {
          activeContext: ctx,
          connection: { connected: isConnected(s, ctx), ...(problem ? { problem } : {}) },
        }
      : {}),
    hosted: s.hosted.mode === 'hosted',
    ...(s.setup.picked !== undefined ? { pickedInSetup: s.setup.picked } : {}),
    ...(s.setup.signInStep ? { signInStep: true } : {}),
    ...(s.setup.choosing ? { choosing: true } : {}),
  }
}

/**
 * The remembered context to open at startup, if any: only when the UI config and the contexts
 * have both arrived, the kubeconfig selects no (valid) current context, and the remembered name
 * is still in the kubeconfig. A kubeconfig `current-context` always wins.
 */
export function startupContextOf(s: StoreData): string | undefined {
  if (s.hosted.mode === 'hosted') return undefined
  if (!s.prefs.loaded || !s.session.contextsLoaded) return undefined
  if (s.session.activeContext !== undefined) return undefined
  const issue = s.session.kubeconfigIssue
  if (issue && issue.kind !== 'NoCurrentContext') return undefined
  const remembered = s.ui.lastSelection?.context
  return remembered !== undefined && s.session.contexts.includes(remembered)
    ? remembered
    : undefined
}
