/**
 * One stack (US1, T050), console first. Under the header: its services and their containers with
 * state, image, ports, uptime and health (FR-007), compact, each container a source for the live
 * log console below them (the first running one by default, or every container). Beside them on
 * wide screens, stacked below on narrow ones: the compose file and `.env`, read-only. Values of
 * secret-looking `.env` keys are masked; an admin can reveal them (audited, FR-021), and they are
 * masked again on leaving the page or after five minutes. Lazy-loaded at `/stacks/:name`; the
 * console and CodeMirror load lazily inside it.
 *
 * Spec 053 adds a second view, the Files tab at `/stacks/:name/files/*` (the volume explorer,
 * lazy-loaded too), for people with the files feature and edit rights.
 */
import { ArrowLeftIcon, PencilSimpleIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { lazy, Suspense, useEffect, useRef, useState, type KeyboardEvent } from 'react'

import { Link } from '@/app/Link'
import type { Params } from '@/app/router'
import { useSession } from '@/app/session-store'
import { useCanReveal, useReveal } from '@/features/editor/reveal-store'
import { filesHref, parseFilesRest, showFilesTab, type FilesLocation } from '@/features/files/model'
import { useUntrusted } from '@/features/network/model'
import { OpenLogs, OpenShell, OpenStats } from '@/features/tabs/OpenTab'
import type { StackContainer as Container, ServiceSummary, Stack, StackDetail as Detail } from '@/protocol/generated'

import { DENY_TEXT } from './argv'
import { DeleteStack } from './DeleteStack'
import { OperationOutput } from './OperationOutput'
import { PreviewDialog } from './PreviewDialog'
import { SecurityChip } from './SecurityChip'
import { ServiceActions, StackActions } from './StackActions'
import { CONSOLE_PANEL_ID, type ConsoleSource } from './StackConsole'
import { StackNotes } from './StackRow'
import { chipFor } from './security'
import { editBlockedReason, elsewhereByStack, formatPorts, formatTime, maskEnv, stackNotes, stateLabel } from './model'
import { StatusBadge } from './StatusBadge'
import { useStacks } from './store'

const CodeView = lazy(() => import('@/features/editor/CodeView'))
const StackConsole = lazy(() => import('./StackConsole'))
const FilesTab = lazy(() => import('@/features/files/FilesTab'))

const STATE_TONE: Record<string, string> = {
  running: 'text-status-ok',
  restarting: 'text-status-warn',
  paused: 'text-status-warn',
  dead: 'text-status-err',
}

const DOT_TONE: Record<string, string> = {
  running: 'bg-status-ok',
  restarting: 'bg-status-warn',
  paused: 'bg-status-warn',
  dead: 'bg-status-err',
}

const HEALTH_TONE = { healthy: 'text-status-ok', starting: 'text-status-warn', unhealthy: 'text-status-err' } as const

/** The console's source: a container by name, or every container of the stack. */
type Pick = { kind: 'container'; name: string } | { kind: 'all' }

/** The first running container, else every container. */
function defaultPick(stack: Stack): Pick {
  const c = stack.services.flatMap((s) => s.containers).find((x) => x.state === 'running')
  return c ? { kind: 'container', name: c.name } : { kind: 'all' }
}

function sourceOf(stack: Stack, pick: Pick): ConsoleSource {
  if (pick.kind === 'container') {
    const security = stack.services.flatMap((s) => s.containers).find((c) => c.name === pick.name)?.security
    return { target: { kind: 'container', container: pick.name }, label: pick.name, container: pick.name, security }
  }
  return { target: { kind: 'stack', stack: stack.name }, label: 'all containers', container: null }
}

/** A state dot; the state is in words beside it too. */
function Dot({ state }: { state: string }) {
  return <span aria-hidden className={`inline-block size-2 shrink-0 rounded-full ${DOT_TONE[state] ?? 'bg-muted-foreground'}`} />
}

const samePick = (a: Pick, b: Pick) => (a.kind === 'all' ? b.kind === 'all' : b.kind === 'container' && b.name === a.name)

/**
 * The console's sources as tabs: every container, then each one with its state and, when it is
 * root or escape, a mark (the chip with its reasons is in the console header and the container
 * row). Arrows, Home and End move between tabs; Enter or Space opens one, so arrowing past a
 * container does not open and close its stream. Scrolls sideways inside itself when they overflow.
 */
function ConsoleTabs({ stack, pick, onPick }: { stack: Stack; pick: Pick; onPick: (p: Pick) => void }) {
  const containers = stack.services.flatMap((s) => s.containers)
  const choices: { pick: Pick; container?: Container }[] = [
    { pick: { kind: 'all' } },
    ...containers.map((c) => ({ pick: { kind: 'container' as const, name: c.name }, container: c })),
  ]
  const refs = useRef<(HTMLButtonElement | null)[]>([])
  const at = Math.max(0, choices.findIndex((c) => samePick(c.pick, pick)))
  const [focus, setFocus] = useState<number | null>(null)
  const current = focus !== null && focus < choices.length ? focus : at

  function onKeyDown(e: KeyboardEvent) {
    const last = choices.length - 1
    const to = { ArrowRight: current + 1, ArrowLeft: current - 1, Home: 0, End: last }[e.key]
    if (to === undefined) return
    e.preventDefault()
    const i = to < 0 ? last : to > last ? 0 : to
    setFocus(i)
    refs.current[i]?.focus()
  }

  return (
    <div
      role="tablist"
      aria-label="Log source"
      onBlur={(e) => !e.currentTarget.contains(e.relatedTarget) && setFocus(null)}
      className="flex min-w-0 shrink-0 overflow-x-auto border-b border-border"
    >
      {choices.map((choice, i) => {
        const c = choice.container
        const selected = i === at
        const chip = chipFor(c?.security?.level)
        const name = c ? c.name : 'All containers'
        return (
          <button
            key={c ? c.id : 'all'}
            ref={(el) => {
              refs.current[i] = el
            }}
            type="button"
            role="tab"
            data-action={c ? 'console-tab' : 'console-pick-all'}
            data-console-tab={c ? c.name : 'all'}
            aria-selected={selected}
            aria-controls={CONSOLE_PANEL_ID}
            tabIndex={i === current ? 0 : -1}
            onKeyDown={onKeyDown}
            title={c ? `${c.name}: ${stateLabel(c.state)}` : "Every container's logs, interleaved"}
            onClick={() => {
              setFocus(null)
              onPick(choice.pick)
            }}
            className={`flex h-7 max-w-52 shrink-0 items-center gap-1.5 border-r border-border px-2 text-xs outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset ${
              selected ? 'border-b-2 border-b-primary bg-background text-foreground' : 'text-muted-foreground'
            }`}
          >
            {c && <Dot state={stateLabel(c.state)} />}
            <span className="truncate font-mono">{name}</span>
            {c && <span className="sr-only">, {stateLabel(c.state)}</span>}
            {chip && (
              <span data-field="security-mark" data-level={chip.level} className={`shrink-0 border px-0.5 text-[0.6rem] leading-3 font-semibold uppercase ${chip.tone}`}>
                {chip.label}
              </span>
            )}
          </button>
        )
      })}
    </div>
  )
}

function ContainerRow({ c, picked, onPick }: { c: Container; picked: boolean; onPick: () => void }) {
  const state = stateLabel(c.state)
  const ports = formatPorts(c.ports)
  return (
    <li
      data-container={c.name}
      data-picked={picked || undefined}
      className={`flex min-w-0 flex-col gap-0.5 border-t border-border px-2 py-1 first:border-t-0 ${picked ? 'bg-accent' : ''}`}
    >
      <div className="flex min-w-0 items-center gap-2">
        <button
          type="button"
          data-action="console-pick"
          aria-pressed={picked}
          title="Show its logs in the console"
          onClick={onPick}
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left text-xs font-medium hover:text-primary"
        >
          <Dot state={state} />
          <span className="min-w-0 truncate">{c.name}</span>
        </button>
        <SecurityChip level={c.security?.level} security={c.security} />
        <span data-field="state" className={`shrink-0 text-xs ${STATE_TONE[state] ?? 'text-muted-foreground'}`}>
          {state}
        </span>
        {c.health !== null && (
          <span data-field="health" className={`shrink-0 text-xs ${HEALTH_TONE[c.health]}`}>
            {c.health}
          </span>
        )}
        <span className="flex shrink-0 items-center">
          <OpenLogs target={{ kind: 'container', container: c.name }} label={c.name} />
          <OpenShell container={c.name} security={c.security} />
          <OpenStats target={{ kind: 'container', container: c.name }} label={c.name} />
        </span>
      </div>
      <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-2 text-xs text-muted-foreground">
        <dt>Image</dt>
        <dd data-field="image" className="min-w-0 truncate text-foreground" title={c.image}>
          {c.image}
        </dd>
        <dt>Uptime</dt>
        <dd data-field="uptime" className="min-w-0 break-words text-foreground">
          {c.status}
        </dd>
        <dt>Ports</dt>
        <dd data-field="ports" className="min-w-0 break-words text-foreground">
          {ports.length > 0 ? ports.join(', ') : 'none'}
        </dd>
        {c.health === null && (
          <>
            <dt>Health</dt>
            <dd data-field="health" className="text-foreground">
              no healthcheck
            </dd>
          </>
        )}
      </dl>
    </li>
  )
}

function ServiceCard({
  service,
  stack,
  pick,
  onPick,
}: {
  service: ServiceSummary
  stack: Stack
  pick: Pick
  onPick: (p: Pick) => void
}) {
  return (
    <section data-service={service.name} aria-label={`Service ${service.name}`} className="min-w-0 border border-border bg-card">
      <header className="flex min-w-0 items-center gap-2 border-b border-border px-2 py-1">
        <h3 className="min-w-0 flex-1 truncate text-xs font-semibold">{service.name}</h3>
        {service.image !== null && <span className="min-w-0 truncate text-xs text-muted-foreground">{service.image}</span>}
        <OpenLogs target={{ kind: 'stack', stack: stack.name, service: service.name }} label={`service ${service.name}`} />
        <ServiceActions stack={stack} service={service.name} />
      </header>
      {service.containers.length === 0 ? (
        <p className="px-2 py-1.5 text-xs text-muted-foreground">No containers.</p>
      ) : (
        <ul>
          {service.containers.map((c) => (
            <ContainerRow
              key={c.id}
              c={c}
              picked={pick.kind === 'container' && pick.name === c.name}
              onPick={() => onPick({ kind: 'container', name: c.name })}
            />
          ))}
        </ul>
      )}
    </section>
  )
}

function CodeFallback() {
  return (
    <p role="status" data-pane-state="loading" className="border border-border p-2 text-xs text-muted-foreground">
      Loading viewer…
    </p>
  )
}

function ConsoleFallback() {
  return (
    <p role="status" data-pane-state="loading" className="flex h-full items-center justify-center border border-border text-xs text-muted-foreground">
      Loading logs…
    </p>
  )
}

const SECTION_TITLE = 'font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase'

function Compose({ detail }: { detail: Detail }) {
  const { stack } = detail
  return (
    <section aria-labelledby="compose-title" className="flex min-w-0 flex-col gap-1">
      <h2 id="compose-title" className={SECTION_TITLE}>
        Compose file <span className="font-mono normal-case">{stack.compose_file}</span>
      </h2>
      {detail.compose_text === null ? (
        <p className="text-xs text-muted-foreground">
          {stack.read_error !== null
            ? `Cannot read: ${stack.read_error}`
            : stack.too_large
              ? 'Too large to load.'
              : 'Not available.'}
        </p>
      ) : (
        <Suspense fallback={<CodeFallback />}>
          <CodeView text={detail.compose_text} language="yaml" label={stack.compose_file} />
        </Suspense>
      )}
    </section>
  )
}

/**
 * `.env`, read-only and masked. An admin can reveal the real values (`RevealEnv`, audited; the
 * server refuses anyone else). They are masked again when the page closes, on "Hide", and when the
 * reveal store's timer runs out.
 */
function Env({ detail }: { detail: Detail }) {
  const { stack } = detail
  const env = detail.env
  const canReveal = useCanReveal()
  const blocked = useUntrusted()
  const revealed = useReveal((s) => s.env[stack.name])
  const reveal = useReveal((s) => s.reveal)
  const forget = useReveal((s) => s.forget)
  useEffect(() => () => forget(stack.name), [stack.name, forget])

  // The server masks by key; masking again here keeps a plain-text reply from ever showing a
  // secret-looking value unless an admin asked for it.
  const masked = env === null ? null : env.kind === 'masked' ? env.text : maskEnv(env.text)
  const hasSecrets = env !== null && (env.kind === 'masked' || masked !== env.text)
  const real = revealed?.state === 'ready' ? revealed.text : null
  const text = real ?? masked
  const offerReveal = text !== null && hasSecrets && canReveal && real === null
  return (
    <section aria-labelledby="env-title" className="flex min-w-0 flex-col gap-1">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <h2 id="env-title" className={`min-w-0 flex-1 ${SECTION_TITLE}`}>
          Environment <span className="font-mono normal-case">.env</span>
        </h2>
        {offerReveal && (
          <Button variant="outline" size="xs" data-action="reveal-env" disabled={revealed?.state === 'asking' || blocked !== null} title={blocked ?? undefined} data-blocked={blocked === null ? undefined : 'untrusted_network'} onClick={() => reveal(stack.name)}>
            {revealed?.state === 'asking' ? 'Revealing…' : 'Reveal'}
          </Button>
        )}
        {real !== null && (
          <Button variant="outline" size="xs" data-action="hide-env" onClick={() => forget(stack.name)}>
            Hide
          </Button>
        )}
      </div>
      {text === null ? (
        <p className="text-xs text-muted-foreground">{stack.has_env ? 'Too large to load.' : 'No .env file.'}</p>
      ) : (
        <>
          <p data-field="env-note" className="text-xs text-muted-foreground">
            {real !== null
              ? 'Showing the real values. They are masked again when you leave, or after 5 minutes.'
              : 'Values of keys that look secret are masked.'}
          </p>
          {offerReveal && (
            <p data-field="reveal-audited" className="text-xs text-muted-foreground">
              Revealing is recorded in the audit log.
            </p>
          )}
          {revealed?.state === 'denied' && (
            <p role="alert" className="text-xs text-status-err">
              {revealed.reason === null ? 'The connection dropped. Try again.' : DENY_TEXT[revealed.reason]}
            </p>
          )}
          <Suspense fallback={<CodeFallback />}>
            <CodeView text={text} language="env" label=".env" />
          </Suspense>
        </>
      )}
    </section>
  )
}

function FilesFallback() {
  return (
    <p role="status" data-pane-state="loading" className="text-xs text-muted-foreground">
      Loading files…
    </p>
  )
}

/**
 * The stack page's views as links (they are URLs, FR-024): the overview and, for people who may
 * use it, Files. Hidden when there is only the overview.
 */
function StackViews({ stack, files }: { stack: string; files: boolean }) {
  const views = [
    { key: 'overview', label: 'Overview', to: `/stacks/${encodeURIComponent(stack)}`, current: !files },
    { key: 'files', label: 'Files', to: filesHref(stack), current: files },
  ]
  return (
    <nav aria-label="Stack views" className="flex min-w-0 shrink-0 border-b border-border">
      {views.map((v) => (
        <Link
          key={v.key}
          to={v.to}
          data-view={v.key}
          aria-current={v.current ? 'page' : undefined}
          className={`flex h-8 items-center px-3 text-xs outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset ${
            v.current ? 'border-b-2 border-b-primary text-foreground' : 'text-muted-foreground'
          }`}
        >
          {v.label}
        </Link>
      ))}
    </nav>
  )
}

/** The Files view's body: the explorer, or why it is not here. */
function FilesView({ stack, rest, shown }: { stack: string; rest: string; shown: boolean }) {
  if (!shown) {
    return (
      <p role="status" data-pane-state="unavailable" className="text-xs text-muted-foreground">
        Files are not available here.
      </p>
    )
  }
  const location: FilesLocation | null = parseFilesRest(rest)
  if (location === null) {
    return (
      <p role="status" data-pane-state="bad-path" className="text-xs text-muted-foreground">
        That is not a path inside this stack.{' '}
        <Link to={filesHref(stack)} className="underline hover:text-primary">
          Show its mounts
        </Link>
      </p>
    )
  }
  return (
    <Suspense fallback={<FilesFallback />}>
      <FilesTab stack={stack} location={location} />
    </Suspense>
  )
}

export default function StackDetail({ params }: { params: Params }) {
  const name = params.name ?? ''
  // `/stacks/:name/files/*` puts the rest of the path in `*`; plain `/stacks/:name` has none.
  const filesRest = params['*']
  const inFiles = filesRest !== undefined
  const filesShown = useSession((s) => showFilesTab(s.me))
  const entry = useStacks((s) => s.details[name])
  const list = useStacks((s) => s.list)
  const openDetail = useStacks((s) => s.openDetail)
  const closeDetail = useStacks((s) => s.closeDetail)
  const load = useStacks((s) => s.load)
  // The person's pick of console source, for this stack; until then (or once it is gone), the default.
  const [picked, setPicked] = useState<{ stack: string; pick: Pick } | null>(null)

  useEffect(() => {
    load()
    openDetail(name)
    return () => closeDetail(name)
  }, [name, load, openDetail, closeDetail])

  const back = (
    // The list is beside the stack on wide screens, so the way back is only for narrow ones.
    <Link to="/" className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-primary lg:hidden">
      <ArrowLeftIcon aria-hidden className="size-3" /> Stacks
    </Link>
  )

  if (entry?.state === 'denied') {
    return (
      <section data-page="stack" className="flex flex-col gap-2 p-2">
        {back}
        <p role="status" className="text-xs text-muted-foreground">
          This stack is not available to you.
        </p>
      </section>
    )
  }
  if (entry?.state !== 'ready') {
    return (
      <section data-page="stack" className="flex flex-col gap-2 p-2">
        {back}
        <p role="status" data-pane-state="loading" className="text-xs text-muted-foreground">
          Loading {name}…
        </p>
      </section>
    )
  }

  const { detail } = entry
  const { stack } = detail
  const elsewhere = elsewhereByStack(list?.foreign ?? []).get(stack.name) ?? []
  const names = new Set(stack.services.flatMap((s) => s.containers.map((c) => c.name)))
  const chosen = picked?.stack === stack.name ? picked.pick : null
  const pick = chosen && (chosen.kind === 'all' || names.has(chosen.name)) ? chosen : defaultPick(stack)
  const onPick = (p: Pick) => setPicked({ stack: stack.name, pick: p })

  return (
    <section data-page="stack" aria-labelledby="stack-title" className="flex min-w-0 flex-col gap-2 p-2 lg:h-full lg:min-h-0">
      <header className="flex min-w-0 shrink-0 flex-col gap-1">
        {back}
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          <h1 id="stack-title" className="min-w-0 truncate font-display text-sm font-semibold tracking-wider">
            {stack.name}
          </h1>
          <StatusBadge status={stack.status} unhealthy={stack.unhealthy} />
          <p className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
            <span>
              {stack.running_services}/{stack.total_services} services up
            </span>
            <span>changed {formatTime(stack.modified_at)}</span>
          </p>
        </div>
        <StackNotes notes={stackNotes(stack, elsewhere)} />
        <div className="flex min-w-0 flex-wrap items-center gap-1">
          <StackActions stack={stack} />
          <EditLink detail={detail} />
          <DeleteStack stack={stack.name} />
          <OpenLogs target={{ kind: 'stack', stack: stack.name }} label={`stack ${stack.name}`} />
          <OpenStats target={{ kind: 'stack', stack: stack.name }} label={`stack ${stack.name}`} />
        </div>
        {filesShown && <StackViews stack={stack.name} files={inFiles} />}
      </header>

      <div className="min-w-0 shrink-0">
        <OperationOutput stack={stack.name} />
      </div>
      <PreviewDialog />

      {inFiles ? (
        <div data-layout="stack-files" className="flex min-w-0 flex-col gap-2 lg:min-h-0 lg:flex-1">
          <FilesView stack={stack.name} rest={filesRest} shown={filesShown} />
        </div>
      ) : (
        <div
          data-layout="stack-body"
          className="grid min-w-0 gap-3 lg:min-h-0 lg:flex-1 lg:grid-cols-[minmax(0,1fr)_24rem] xl:grid-cols-[minmax(0,1fr)_28rem] 2xl:grid-cols-[minmax(0,1fr)_36rem]"
        >
          <div data-column="console" className="flex min-w-0 flex-col gap-2 lg:min-h-0">
            <section aria-labelledby="services-title" className="flex min-w-0 shrink-0 flex-col gap-1 lg:max-h-[45%] lg:overflow-auto">
              <h2 id="services-title" className={SECTION_TITLE}>
                Services
              </h2>
              {stack.services.length === 0 ? (
                <p className="text-xs text-muted-foreground">No services.</p>
              ) : (
                <div className="grid min-w-0 gap-2 md:grid-cols-2 2xl:grid-cols-3">
                  {stack.services.map((s) => (
                    <ServiceCard key={s.name} service={s} stack={stack} pick={pick} onPick={onPick} />
                  ))}
                </div>
              )}
            </section>
            <div className="h-96 min-w-0 shrink-0 lg:h-auto lg:min-h-64 lg:flex-1 lg:shrink">
              <Suspense fallback={<ConsoleFallback />}>
                <StackConsole
                  key={stack.name}
                  stack={stack.name}
                  source={sourceOf(stack, pick)}
                  tabs={<ConsoleTabs stack={stack} pick={pick} onPick={onPick} />}
                />
              </Suspense>
            </div>
          </div>

          <aside aria-label="Compose file and .env" data-column="files" className="flex min-w-0 flex-col gap-3 lg:min-h-0 lg:overflow-auto">
            <Compose detail={detail} />
            <Env detail={detail} />
          </aside>
        </div>
      )}
    </section>
  )
}

/** The way into the editor (US3): shown while editing is on and the files can be edited here. */
function EditLink({ detail }: { detail: Detail }) {
  const editOn = useSession((s) => s.me?.features.edit ?? false)
  if (!editOn || editBlockedReason(detail) !== null) return null
  return (
    <Link
      to={`/stacks/${encodeURIComponent(detail.stack.name)}/edit`}
      data-action="edit"
      className="inline-flex h-7 items-center gap-1 border border-input px-2 text-xs hover:border-primary hover:text-primary"
    >
      <PencilSimpleIcon aria-hidden className="size-3.5" />
      Edit
    </Link>
  )
}
