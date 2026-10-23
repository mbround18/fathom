/**
 * First-run and connection-failure pane state: what went wrong, in words, and the next step
 * (create a local kind cluster, pick another context, reload the kubeconfig, retry). Replaces
 * the raw error walls, endless skeletons and the bare "No kubeconfig" notice these cases used
 * to produce. The verbatim backend error stays one click away in "Details".
 *
 * 043: the "no usable kubeconfig" and "pick a context" cases are the app-wide setup screen
 * (`components/setup/SetupScreen.tsx`), so panes normally only show the `connection` kind here.
 * The other kinds still render where the setup screen does not apply (a settings page opened
 * from setup, hosted mode).
 */
import {
  ArrowsClockwiseIcon,
  FileXIcon,
  KeyIcon,
  ListBulletsIcon,
  PlugsIcon,
  PlusIcon,
} from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import {
  connectionProblemCopy,
  kubeconfigIssueCopy,
  pickCopy,
  reloadOutcome,
  type Copy,
  type FirstRun,
} from '@/lib/first-run'
import { SECTION } from '@/protocol/commands'
import { useStore } from '@/store'

import { StateFrame } from './PaneStates'
import { useReload } from './use-reload'

/** More than this many contexts are left to the header's context picker. */
const MAX_CONTEXT_BUTTONS = 8

function copyFor(state: FirstRun): Copy {
  switch (state.kind) {
    case 'kubeconfig':
      return kubeconfigIssueCopy(state.issue)
    case 'pick':
      return pickCopy(state.issue)
    case 'connection':
      return connectionProblemCopy(state.context, state.problem)
  }
}

function iconFor(state: FirstRun) {
  if (state.kind === 'kubeconfig') return FileXIcon
  if (state.kind === 'pick') return ListBulletsIcon
  const k = state.problem.kind
  return k === 'Unauthorized' || k === 'Forbidden' || k === 'Tls' ? KeyIcon : PlugsIcon
}

export function FirstRunState({
  state,
  inSettings = false,
}: {
  state: FirstRun
  /** Rendered in Cluster settings, which offers its own "Create a local cluster" link. */
  inSettings?: boolean
}) {
  const hosted = useStore((s) => s.hosted.mode === 'hosted')
  const contexts = useStore((s) => s.session.contexts)
  const statuses = useStore((s) => s.session.contextStatuses)
  const navigate = useStore((s) => s.navigate)
  const selectContext = useStore((s) => s.selectContext)
  const retryConnection = useStore((s) => s.retryConnection)
  const send = useStore((s) => s.send)
  const kubeconfigPaths = useStore((s) => s.session.kubeconfigPaths)
  const kubeconfigIssue = useStore((s) => s.session.kubeconfigIssue)
  const reload = useReload(send)
  const setShell = useStore((s) => s.setShell)
  const widthClass = useStore((s) => s.layout.widthClass)
  // 043: the header context picker (the scope sheet on phone, where the picker lives).
  const chooseAnother = () =>
    setShell(widthClass === 'phone' ? { scopeOpen: true } : { contextPickerOpen: true })
  const paths =
    kubeconfigPaths.length > 0
      ? kubeconfigPaths
      : state.kind === 'connection'
        ? []
        : (state.issue?.paths ?? [])

  const { title, detail } = copyFor(state)
  const active = state.kind === 'connection' ? state.context : undefined
  const others = contexts.filter((c) => c !== active)
  const shown = others.slice(0, MAX_CONTEXT_BUTTONS)
  const verbatim =
    state.kind === 'connection'
      ? state.problem.detail
      : state.kind === 'kubeconfig' && state.issue.detail
        ? state.issue.detail
        : undefined
  const retrying = active !== undefined && statuses[active]?.status === 'Connecting'

  const actions = (
    <div className="flex max-w-prose flex-col items-center gap-3">
      <div className="flex flex-wrap justify-center gap-2">
        {state.kind === 'connection' && (
          <Button
            variant="outline"
            size="sm"
            data-testid="first-run-retry"
            disabled={retrying}
            onClick={retryConnection}
          >
            <ArrowsClockwiseIcon data-icon="inline-start" /> {retrying ? 'Retrying…' : 'Retry'}
          </Button>
        )}
        {state.kind === 'connection' && !hosted && contexts.length > 1 && (
          <Button
            variant="outline"
            size="sm"
            data-testid="first-run-choose-context"
            onClick={chooseAnother}
          >
            <ListBulletsIcon data-icon="inline-start" /> Choose another context
          </Button>
        )}
        {state.kind !== 'connection' && !hosted && (
          <Button
            variant="outline"
            size="sm"
            data-testid="first-run-reload"
            data-pending={reload.pending || undefined}
            aria-busy={reload.pending}
            disabled={reload.pending}
            onClick={reload.reload}
          >
            <ArrowsClockwiseIcon
              data-icon="inline-start"
              className={reload.pending ? 'animate-spin motion-reduce:animate-none' : undefined}
            />{' '}
            {reload.pending ? 'Reloading…' : 'Reload kubeconfig'}
          </Button>
        )}
        {!hosted && !inSettings && state.kind !== 'pick' && (
          <Button
            size="sm"
            data-testid="first-run-create-kind"
            onClick={() => navigate(SECTION.localClusters)}
          >
            <PlusIcon data-icon="inline-start" /> Create a local cluster
          </Button>
        )}
      </div>
      {state.kind !== 'connection' && !hosted && (reload.done || reload.timedOut) && (
        <p role="status" data-testid="first-run-reload-outcome" className="text-xs">
          {reload.timedOut
            ? 'No answer from the backend yet. Try again in a moment.'
            : reloadOutcome({
                issue: kubeconfigIssue,
                paths: kubeconfigPaths,
                contextCount: contexts.length,
              })}
        </p>
      )}
      {state.kind !== 'connection' && !hosted && paths.length > 0 && (
        <p data-testid="first-run-paths" className="text-xs text-muted-foreground">
          Kubeconfig:{' '}
          {paths.map((p, i) => (
            <span key={p}>
              {i > 0 && ', '}
              <code className="font-mono break-all">{p}</code>
            </span>
          ))}
        </p>
      )}
      {shown.length > 0 && (
        <div className="flex flex-col items-center gap-1">
          <p className="text-xs text-muted-foreground">
            {state.kind === 'pick' ? 'Contexts in your kubeconfig:' : 'Or switch to:'}
          </p>
          <ul className="flex flex-wrap justify-center gap-1" data-testid="first-run-contexts">
            {shown.map((name) => (
              <li key={name}>
                <Button
                  variant="ghost"
                  size="sm"
                  className="font-mono"
                  data-testid="first-run-context"
                  onClick={() => selectContext(name)}
                >
                  {name}
                </Button>
              </li>
            ))}
          </ul>
          {others.length > shown.length && (
            <p className="text-xs text-muted-foreground">
              {others.length - shown.length} more in the context picker.
            </p>
          )}
        </div>
      )}
      {verbatim && (
        <details className="w-full text-left text-xs text-muted-foreground">
          <summary className="cursor-pointer text-center">Details</summary>
          <pre
            data-testid="first-run-detail"
            className="mt-1 max-h-40 overflow-auto rounded-sm border border-border p-2 break-all whitespace-pre-wrap"
          >
            {verbatim}
          </pre>
        </details>
      )}
    </div>
  )

  return (
    <div data-testid="first-run" data-first-run={state.kind} className="contents">
      <StateFrame
        kind={`first-run-${state.kind}`}
        icon={iconFor(state)}
        tone={state.kind === 'connection' ? 'warn' : 'muted'}
        title={title}
        detail={detail}
        action={actions}
        trace={!inSettings}
      />
    </div>
  )
}
