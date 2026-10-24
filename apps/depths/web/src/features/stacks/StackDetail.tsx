/**
 * One stack (US1, T050): services and their containers with state, image, ports, uptime and
 * health (FR-007), then the compose file and `.env`, read-only. Values of secret-looking `.env`
 * keys are masked. Lazy-loaded at `/stacks/:name`; CodeMirror loads lazily inside it.
 */
import { ArrowLeftIcon, PencilSimpleIcon } from '@phosphor-icons/react'
import { lazy, Suspense, useEffect } from 'react'

import { Link } from '@/app/Link'
import type { Params } from '@/app/router'
import { useSession } from '@/app/session-store'
import { OpenLogs, OpenShell, OpenStats } from '@/features/tabs/OpenTab'
import type { Container, ServiceSummary, Stack, StackDetail as Detail } from '@/protocol/generated'

import { DeleteStack } from './DeleteStack'
import { OperationOutput } from './OperationOutput'
import { PreviewDialog } from './PreviewDialog'
import { ServiceActions, StackActions } from './StackActions'
import { StackNotes } from './StackRow'
import { editBlockedReason, elsewhereByStack, formatPorts, formatTime, maskEnv, stackNotes, stateLabel } from './model'
import { StatusBadge } from './StatusBadge'
import { useStacks } from './store'

const CodeView = lazy(() => import('@/features/editor/CodeView'))

const STATE_TONE: Record<string, string> = {
  running: 'text-status-ok',
  restarting: 'text-status-warn',
  paused: 'text-status-warn',
  dead: 'text-status-err',
}

const HEALTH_TONE = { healthy: 'text-status-ok', starting: 'text-status-warn', unhealthy: 'text-status-err' } as const

function ContainerRow({ c }: { c: Container }) {
  const state = stateLabel(c.state)
  const ports = formatPorts(c.ports)
  return (
    <li data-container={c.name} className="flex min-w-0 flex-col gap-0.5 border-t border-border px-2 py-1.5 first:border-t-0">
      <div className="flex min-w-0 items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-xs font-medium">{c.name}</span>
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
          <OpenShell container={c.name} />
          <OpenStats target={{ kind: 'container', container: c.name }} label={c.name} />
        </span>
      </div>
      <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-2 text-xs text-muted-foreground">
        <dt>Image</dt>
        <dd data-field="image" className="min-w-0 break-all text-foreground/90">
          {c.image}
        </dd>
        <dt>Uptime</dt>
        <dd data-field="uptime" className="min-w-0 break-words text-foreground/90">
          {c.status}
        </dd>
        <dt>Ports</dt>
        <dd data-field="ports" className="min-w-0 break-words text-foreground/90">
          {ports.length > 0 ? ports.join(', ') : 'none'}
        </dd>
        {c.health === null && (
          <>
            <dt>Health</dt>
            <dd data-field="health" className="text-foreground/90">
              no healthcheck
            </dd>
          </>
        )}
      </dl>
    </li>
  )
}

function ServiceCard({ service, stack }: { service: ServiceSummary; stack: Stack }) {
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
            <ContainerRow key={c.id} c={c} />
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

function Files({ detail }: { detail: Detail }) {
  const { stack } = detail
  const env = detail.env
  // The server masks by key; masking again here keeps a plain-text reply from ever showing a
  // secret-looking value in the read-only view (revealing is an explicit, audited action).
  const envText = env === null ? null : env.kind === 'masked' ? env.text : maskEnv(env.text)
  return (
    <>
      <section aria-labelledby="compose-title" className="flex min-w-0 flex-col gap-1">
        <h2 id="compose-title" className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
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
      <section aria-labelledby="env-title" className="flex min-w-0 flex-col gap-1">
        <h2 id="env-title" className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          Environment <span className="font-mono normal-case">.env</span>
        </h2>
        {envText === null ? (
          <p className="text-xs text-muted-foreground">{stack.has_env ? 'Too large to load.' : 'No .env file.'}</p>
        ) : (
          <>
            <p className="text-xs text-muted-foreground">Values of keys that look secret are masked.</p>
            <Suspense fallback={<CodeFallback />}>
              <CodeView text={envText} language="env" label=".env" />
            </Suspense>
          </>
        )}
      </section>
    </>
  )
}

export default function StackDetail({ params }: { params: Params }) {
  const name = params.name ?? ''
  const entry = useStacks((s) => s.details[name])
  const list = useStacks((s) => s.list)
  const openDetail = useStacks((s) => s.openDetail)
  const closeDetail = useStacks((s) => s.closeDetail)
  const load = useStacks((s) => s.load)

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

  return (
    <section data-page="stack" aria-labelledby="stack-title" className="flex min-w-0 flex-col gap-3 p-2">
      <header className="flex min-w-0 flex-col gap-1">
        {back}
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <h1 id="stack-title" className="min-w-0 truncate font-display text-sm font-semibold tracking-wider">
            {stack.name}
          </h1>
          <StatusBadge status={stack.status} unhealthy={stack.unhealthy} />
        </div>
        <p className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
          <span>
            {stack.running_services}/{stack.total_services} services up
          </span>
          <span>changed {formatTime(stack.modified_at)}</span>
        </p>
        <StackNotes notes={stackNotes(stack, elsewhere)} />
        <div className="flex min-w-0 flex-wrap items-center gap-1">
          <StackActions stack={stack} />
          <EditLink detail={detail} />
          <DeleteStack stack={stack.name} />
          <OpenLogs target={{ kind: 'stack', stack: stack.name }} label={`stack ${stack.name}`} />
          <OpenStats target={{ kind: 'stack', stack: stack.name }} label={`stack ${stack.name}`} />
        </div>
      </header>

      <OperationOutput stack={stack.name} />
      <PreviewDialog />

      <section aria-labelledby="services-title" className="flex min-w-0 flex-col gap-1">
        <h2 id="services-title" className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          Services
        </h2>
        {stack.services.length === 0 ? (
          <p className="text-xs text-muted-foreground">No services.</p>
        ) : (
          <div className="grid min-w-0 gap-2 md:grid-cols-2 xl:grid-cols-3">
            {stack.services.map((s) => (
              <ServiceCard key={s.name} service={s} stack={stack} />
            ))}
          </div>
        )}
      </section>

      <div className="grid min-w-0 gap-3 lg:grid-cols-2">
        <Files detail={detail} />
      </div>
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
