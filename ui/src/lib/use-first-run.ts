import { useMemo } from 'react'

import { useStore } from '@/store'

import { firstRunOf, type FirstRun } from './first-run'

/** The first-run / connection-failure state panes should show, or `undefined` to render normally. */
export function useFirstRun(): FirstRun | undefined {
  const contextsLoaded = useStore((s) => s.session.contextsLoaded)
  const contextCount = useStore((s) => s.session.contexts.length)
  const activeContext = useStore((s) => s.session.activeContext)
  const issue = useStore((s) => s.session.kubeconfigIssue)
  const problem = useStore((s) =>
    s.session.activeContext === undefined
      ? undefined
      : s.session.connectionProblems[s.session.activeContext],
  )
  return useMemo(
    () => firstRunOf({ contextsLoaded, contextCount, activeContext, issue, problem }),
    [contextsLoaded, contextCount, activeContext, issue, problem],
  )
}
