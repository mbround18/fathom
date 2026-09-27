/**
 * 043 US1: the setup screen. When Fathom has no usable kubeconfig, or the kubeconfig selects no
 * context, this one screen replaces the navigator, the panes and the dock (the top bar and the
 * status bar stay). It names the kubeconfig files and every context verbatim, reloads the
 * kubeconfig with an answer every time (feedback #1), and never writes the kubeconfig: a pick is
 * remembered in Fathom's own UI config.
 */
import {
  ArrowsClockwiseIcon,
  CircleNotchIcon,
  FileXIcon,
  KeyIcon,
  ListBulletsIcon,
  MagnifyingGlassIcon,
  PlugsIcon,
  PlusIcon,
  SignInIcon,
} from '@phosphor-icons/react'
import { useVirtualizer } from '@tanstack/react-virtual'
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react'

import { useReload } from '@/components/shell/use-reload'
import { Button } from '@/components/ui/button'
import {
  connectionProblemCopy,
  kubeconfigIssueCopy,
  pickCopy,
  reloadOutcome,
} from '@/lib/first-run'
import { cn } from '@/lib/utils'
import { SECTION } from '@/protocol/commands'
import { useStore } from '@/store'
import { CircuitTrace } from '@/theme/CircuitTrace'

import type { SetupState } from './setup-state'
import { SetupSignIn } from './SetupSignIn'

/** Longer context lists render only the rows in view. */
export const VIRTUALIZE_ABOVE = 200
const ROW_PX = 36

export function SetupScreen({ state }: { state: SetupState }) {
  // Owned here, above every state view, so a kubeconfig reload keeps what was typed.
  const [filter, setFilter] = useState('')
  return (
    <div
      data-testid="setup-screen"
      data-setup-state={state.kind}
      className="relative flex min-h-0 flex-1 flex-col overflow-y-auto"
    >
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-4 py-6 sm:px-8 sm:py-10">
        {state.kind === 'kubeconfig' || state.kind === 'pick' ? (
          <SetupPick state={state} filter={filter} onFilter={setFilter} />
        ) : state.kind === 'sign-in' ? (
          <SetupSignIn context={state.context} />
        ) : (
          <SetupConnect state={state} />
        )}
      </div>
    </div>
  )
}

/** The eyebrow, title and one-paragraph explanation every state view starts with. */
function Heading({
  icon: Icon,
  title,
  detail,
  tone = 'muted',
}: {
  icon: typeof FileXIcon
  title: string
  detail?: ReactNode
  tone?: 'muted' | 'warn'
}) {
  return (
    <header className="flex flex-col gap-2">
      <p className="flex items-center gap-2 font-display text-[0.65rem] tracking-[0.3em] text-accent-neon/80 uppercase">
        <Icon
          aria-hidden="true"
          className={cn('size-4', tone === 'warn' ? 'text-status-warn' : 'text-accent-neon')}
        />
        Set up Fathom
      </p>
      <h2 className="font-display text-lg font-semibold tracking-wider uppercase">{title}</h2>
      {detail && <p className="max-w-prose text-xs text-muted-foreground">{detail}</p>}
    </header>
  )
}

function Details({ text }: { text: string }) {
  return (
    <details className="text-xs text-muted-foreground">
      <summary className="cursor-pointer">Details</summary>
      <pre
        data-testid="setup-detail"
        className="mt-1 max-h-40 overflow-auto border border-border p-2 break-all whitespace-pre-wrap"
      >
        {text}
      </pre>
    </details>
  )
}

/** Kubeconfig paths, "Reload kubeconfig" with its outcome, and "Create a local cluster". */
function KubeconfigActions({ offerKind }: { offerKind: boolean }) {
  const send = useStore((s) => s.send)
  const navigate = useStore((s) => s.navigate)
  const paths = useStore((s) => s.session.kubeconfigPaths)
  const issue = useStore((s) => s.session.kubeconfigIssue)
  const contextCount = useStore((s) => s.session.contexts.length)
  const reload = useReload(send)
  const shown = paths.length > 0 ? paths : (issue?.paths ?? [])

  return (
    <div className="flex flex-col gap-3">
      {shown.length > 0 && (
        <div data-testid="setup-paths" className="flex flex-col gap-1 text-xs">
          <p className="text-muted-foreground">
            {shown.length === 1 ? 'Kubeconfig file' : 'Kubeconfig files'}
          </p>
          <ul className="flex flex-col gap-0.5">
            {shown.map((p) => (
              <li key={p}>
                <code className="font-mono break-all">{p}</code>
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          data-testid="setup-reload"
          data-pending={reload.pending || undefined}
          aria-busy={reload.pending}
          disabled={reload.pending}
          onClick={reload.reload}
        >
          <ArrowsClockwiseIcon
            data-icon="inline-start"
            className={reload.pending ? 'animate-spin motion-reduce:animate-none' : undefined}
          />
          {reload.pending ? 'Reloading…' : 'Reload kubeconfig'}
        </Button>
        {offerKind && (
          <Button
            size="sm"
            variant="outline"
            data-testid="setup-create-kind"
            onClick={() => navigate(SECTION.localClusters)}
          >
            <PlusIcon data-icon="inline-start" /> Create a local cluster
          </Button>
        )}
      </div>
      {(reload.done || reload.timedOut) && (
        <p role="status" data-testid="setup-reload-outcome" className="text-xs">
          {reload.timedOut
            ? 'No answer from the backend yet. Try again in a moment.'
            : reloadOutcome({ issue, paths, contextCount })}
        </p>
      )}
    </div>
  )
}

function SetupPick({
  state,
  filter,
  onFilter,
}: {
  state: Extract<SetupState, { kind: 'kubeconfig' | 'pick' }>
  filter: string
  onFilter: (v: string) => void
}) {
  const hosted = useStore((s) => s.hosted.mode === 'hosted')
  const issue = useStore((s) => s.session.kubeconfigIssue)
  const choosing = useStore((s) => s.setup.choosing === true)

  if (state.kind === 'kubeconfig') {
    const copy = kubeconfigIssueCopy(state.issue)
    return (
      <section className="flex max-w-2xl flex-col gap-5">
        <Heading icon={FileXIcon} title={copy.title} detail={copy.detail} />
        <KubeconfigActions offerKind={!hosted} />
        {state.issue.detail && <Details text={state.issue.detail} />}
        <CircuitTrace className="opacity-60" />
      </section>
    )
  }

  const copy = choosing
    ? { title: 'Choose another context', detail: 'Pick the cluster to open instead.' }
    : pickCopy(issue)
  return (
    <div className="grid grid-cols-1 gap-8 md:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
      <section data-testid="setup-intro" className="flex flex-col gap-5">
        <Heading icon={ListBulletsIcon} title={copy.title} detail={copy.detail} />
        <p className="max-w-prose text-xs text-muted-foreground">
          Fathom opens the context you pick and remembers it for next time. Your kubeconfig is not
          changed.
        </p>
        <KubeconfigActions offerKind={!hosted} />
      </section>
      <ContextList contexts={state.contexts} filter={filter} onFilter={onFilter} />
    </div>
  )
}

function ContextList({
  contexts,
  filter,
  onFilter,
}: {
  contexts: readonly string[]
  filter: string
  onFilter: (v: string) => void
}) {
  const pick = useStore((s) => s.pickInSetup)
  const problems = useStore((s) => s.session.connectionProblems)
  const listId = useId()
  const optionId = (i: number) => `${listId}-opt-${i}`
  const q = filter.trim().toLowerCase()
  const shown = useMemo(
    () => (q === '' ? contexts : contexts.filter((c) => c.toLowerCase().includes(q))),
    [contexts, q],
  )
  const [active, setActive] = useState(0)
  const current = Math.min(active, Math.max(shown.length - 1, 0))
  const scrollRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  // Focus the filter once, so typing narrows the list straight away.
  useEffect(() => inputRef.current?.focus(), [])
  const virtual = shown.length > VIRTUALIZE_ABOVE
  const virtualizer = useVirtualizer({
    count: virtual ? shown.length : 0,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_PX,
    overscan: 12,
  })

  useEffect(() => {
    if (virtual) virtualizer.scrollToIndex(current)
    else {
      const el = document.getElementById(optionId(current))
      // jsdom has no scrollIntoView.
      if (el && typeof el.scrollIntoView === 'function') el.scrollIntoView({ block: 'nearest' })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- scroll only when the active row moves
  }, [current, virtual])

  const onKeyDown = (e: KeyboardEvent) => {
    if (shown.length === 0) return
    const move: Record<string, number> = {
      ArrowDown: current + 1,
      ArrowUp: current - 1,
      Home: 0,
      End: shown.length - 1,
      PageDown: current + 8,
      PageUp: current - 8,
    }
    const next = move[e.key]
    if (next !== undefined) {
      e.preventDefault()
      setActive(Math.max(0, Math.min(shown.length - 1, next)))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const name = shown[current]
      if (name !== undefined) pick(name)
    }
  }

  const option = (name: string, i: number, style?: React.CSSProperties) => {
    const problem = problems[name]
    return (
      <div
        key={name}
        id={optionId(i)}
        role="option"
        aria-selected={i === current}
        data-testid="setup-context"
        data-context={name}
        style={style}
        onMouseEnter={() => setActive(i)}
        tabIndex={-1}
        onClick={() => pick(name)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            pick(name)
          }
        }}
        className={cn(
          'flex h-9 cursor-pointer items-center gap-2 border-l-2 px-3 font-mono text-xs',
          i === current
            ? 'border-accent-neon bg-accent-neon/10 text-foreground'
            : 'border-transparent text-foreground/90 hover:bg-muted/40',
        )}
      >
        <PlugsIcon aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate">{name}</span>
        {problem && (
          <span className="shrink-0 font-sans text-[0.65rem] text-status-warn">
            could not connect
          </span>
        )}
      </div>
    )
  }

  return (
    <section data-testid="setup-contexts" className="flex min-w-0 flex-col gap-2">
      <label className="relative flex items-center">
        <span className="sr-only">Filter contexts</span>
        <MagnifyingGlassIcon
          aria-hidden="true"
          className="pointer-events-none absolute left-2.5 size-3.5 text-muted-foreground"
        />
        <input
          type="search"
          role="searchbox"
          aria-label="Filter contexts"
          aria-controls={listId}
          aria-activedescendant={shown.length > 0 ? optionId(current) : undefined}
          placeholder={`Filter ${contexts.length} context${contexts.length === 1 ? '' : 's'}`}
          value={filter}
          ref={inputRef}
          onChange={(e) => {
            onFilter(e.target.value)
            setActive(0)
          }}
          onKeyDown={onKeyDown}
          className="h-[max(2.25rem,var(--touch-target))] w-full border border-border bg-background/40 pr-2 pl-8 font-mono text-xs outline-none placeholder:text-muted-foreground focus-visible:border-accent-neon focus-visible:ring-1 focus-visible:ring-ring"
        />
      </label>
      <div
        ref={scrollRef}
        id={listId}
        role="listbox"
        aria-label="Contexts"
        tabIndex={0}
        aria-activedescendant={shown.length > 0 ? optionId(current) : undefined}
        onKeyDown={onKeyDown}
        className="max-h-[min(60vh,28rem)] min-h-24 overflow-y-auto border border-border bg-card/40 outline-none focus-visible:ring-1 focus-visible:ring-ring"
      >
        {shown.length === 0 ? (
          <p className="p-3 text-xs text-muted-foreground" data-testid="setup-no-match">
            No context matches “{filter.trim()}”.
          </p>
        ) : virtual ? (
          <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
            {virtualizer.getVirtualItems().map((v) =>
              option(shown[v.index] ?? '', v.index, {
                position: 'absolute',
                top: 0,
                left: 0,
                right: 0,
                transform: `translateY(${v.start}px)`,
              }),
            )}
          </div>
        ) : (
          shown.map((name, i) => option(name, i))
        )}
      </div>
      <p className="text-[0.65rem] text-muted-foreground">
        {shown.length === contexts.length
          ? `${contexts.length} context${contexts.length === 1 ? '' : 's'} in your kubeconfig.`
          : `${shown.length} of ${contexts.length} contexts.`}{' '}
        Use the arrow keys and Enter, or click one.
      </p>
    </section>
  )
}

/** A context picked in setup is connecting, or could not connect. */
function SetupConnect({
  state,
}: {
  state: Extract<SetupState, { kind: 'connecting' | 'failed' }>
}) {
  const retry = useStore((s) => s.retryConnection)
  const setSetup = useStore((s) => s.setSetup)
  const signInDone = useStore((s) => s.setup.signInDone === true)
  const statuses = useStore((s) => s.session.contextStatuses)
  const contextCount = useStore((s) => s.session.contexts.length)
  const unauthorized = state.kind === 'failed' && state.problem.kind === 'Unauthorized'

  // A 401 offers the optional sign-in step once per pick (spec US2 scenario 1).
  useEffect(() => {
    if (unauthorized && !signInDone) setSetup({ signInStep: true })
  }, [unauthorized, signInDone, setSetup])

  const chooseAnother =
    contextCount > 1 ? (
      <Button
        variant="outline"
        size="sm"
        data-testid="setup-choose-another"
        onClick={() => setSetup({ choosing: true, picked: undefined, signInStep: false })}
      >
        <ListBulletsIcon data-icon="inline-start" /> Choose another context
      </Button>
    ) : null

  if (state.kind === 'connecting') {
    return (
      <section className="flex max-w-2xl flex-col gap-5" aria-busy="true">
        <Heading
          icon={CircleNotchIcon}
          title={`Connecting to ${state.context}`}
          detail="Fathom is contacting the cluster's API server."
        />
        <p role="status" className="flex items-center gap-2 text-xs text-muted-foreground">
          <CircleNotchIcon
            aria-hidden="true"
            className="size-4 animate-spin text-accent-neon motion-reduce:animate-none"
          />
          Waiting for <span className="font-mono">{state.context}</span> to answer…
        </p>
        <div className="flex flex-wrap gap-2">{chooseAnother}</div>
      </section>
    )
  }

  const copy = connectionProblemCopy(state.context, state.problem)
  const k = state.problem.kind
  const credentials = k === 'Unauthorized' || k === 'Forbidden' || k === 'Tls'
  const retrying = statuses[state.context]?.status === 'Connecting'
  return (
    <section className="flex max-w-2xl flex-col gap-5" data-testid="setup-failed">
      <Heading
        icon={credentials ? KeyIcon : PlugsIcon}
        title={copy.title}
        detail={copy.detail}
        tone="warn"
      />
      <div className="flex flex-wrap gap-2">
        <Button size="sm" data-testid="setup-retry" disabled={retrying} onClick={retry}>
          <ArrowsClockwiseIcon data-icon="inline-start" /> {retrying ? 'Retrying…' : 'Retry'}
        </Button>
        {(k === 'Unauthorized' || k === 'Forbidden') && (
          <Button
            variant="outline"
            size="sm"
            data-testid="setup-sign-in"
            onClick={() => setSetup({ signInStep: true })}
          >
            <SignInIcon data-icon="inline-start" /> Sign in
          </Button>
        )}
        {chooseAnother}
      </div>
      {state.problem.detail && <Details text={state.problem.detail} />}
    </section>
  )
}

/** Shown in place of the setup screen while a clusterless page is open from it. */
export function SetupDetourBar() {
  const setSetup = useStore((s) => s.setSetup)
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-border bg-card px-3 py-1.5 text-xs">
      <Button
        variant="ghost"
        size="sm"
        data-testid="setup-back"
        onClick={() => setSetup({ detour: undefined })}
      >
        ← Back to setup
      </Button>
      <span className="text-muted-foreground">No context is open yet.</span>
    </div>
  )
}
