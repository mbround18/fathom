/**
 * `/new` (US3, T062): create a stack, admin only and only while editing is on. The name is
 * checked here for shape (a compose project name) and by the server for shape and uniqueness; the
 * server's reason is shown as it gave it (FR-022). Creating goes through the same preview as a
 * save (`PreviewSave` without `base_hash`), alone or with a deploy.
 */
import { ArrowLeftIcon, PlusIcon, RocketLaunchIcon } from '@phosphor-icons/react'
import { Button, Input } from '@companion/ui'
import { lazy, Suspense, useEffect, useId, useMemo, useState, type ReactNode } from 'react'

import { Link } from '@/app/Link'
import { useSession } from '@/app/session-store'
import { yamlProblems } from '@/features/editor/problems'
import { useUnsavedGuard } from '@/features/editor/useUnsavedGuard'

import { OperationOutput } from './OperationOutput'
import { useOps } from './ops-store'
import { PreviewDialog } from './PreviewDialog'
import { useStacks } from './store'

const ComposeEditor = lazy(() => import('@/features/editor/ComposeEditor'))
const EnvEditor = lazy(() => import('@/features/editor/EnvEditor'))

const TEMPLATE = 'services:\n  app:\n    image: \n'
/** A compose project name: lowercase letters, digits, `-` and `_`, starting with a letter or digit. */
const NAME = /^[a-z0-9][a-z0-9_-]*$/

function nameProblem(name: string): string | null {
  if (name === '') return null
  if (!NAME.test(name)) return 'Use lowercase letters, digits, - and _, starting with a letter or digit.'
  return null
}

function Loading() {
  return (
    <p role="status" data-pane-state="loading" className="border border-border p-2 text-xs text-muted-foreground">
      Loading editor…
    </p>
  )
}

function Form() {
  const [name, setName] = useState('')
  const [compose, setCompose] = useState(TEMPLATE)
  const [env, setEnv] = useState('')
  const [awaiting, setAwaiting] = useState<{ name: string; before: string | null } | null>(null)
  const nameId = useId()

  const flow = useOps((s) => s.flow)
  const ops = useOps((s) => s.ops)
  const requestPreview = useOps((s) => s.requestPreview)
  const taken = useStacks((s) => s.list?.stacks.some((x) => x.name === name) ?? false)

  const yamlErrors = useMemo(() => yamlProblems(compose), [compose])
  const badName = nameProblem(name)
  // Created once the confirmed create's operation has started: its output replaces the form.
  const run = awaiting ? ops[awaiting.name] : undefined
  const created = awaiting && run?.kind === 'create' && run.operationId !== awaiting.before ? awaiting.name : null
  const dirty = created === null && (name !== '' || compose !== TEMPLATE || env !== '')
  useUnsavedGuard(dirty)

  const asking = flow?.request.kind === 'save' && (flow.phase === 'previewing' || flow.phase === 'confirming')
  const canCreate = name !== '' && badName === null && yamlErrors.length === 0 && !asking && created === null

  const create = (deploy: boolean): void => {
    setAwaiting({ name, before: ops[name]?.operationId ?? null })
    requestPreview({ kind: 'save', draft: { stack: name, compose, ...(env !== '' ? { env } : {}) }, deploy })
  }

  if (created !== null) {
    return (
      <div className="flex min-w-0 flex-col gap-2">
        <OperationOutput stack={created} />
        <Link to={`/stacks/${encodeURIComponent(created)}`} className="text-xs text-primary hover:underline">
          Open {created}
        </Link>
      </div>
    )
  }

  return (
    <>
      <div className="flex min-w-0 flex-col gap-1">
        <label htmlFor={nameId} className="text-xs font-medium">
          Name
        </label>
        <Input
          id={nameId}
          value={name}
          autoComplete="off"
          spellCheck={false}
          aria-invalid={badName !== null}
          aria-describedby={`${nameId}-hint`}
          onChange={(e) => setName(e.target.value)}
          className="max-w-xs"
        />
        <p id={`${nameId}-hint`} data-field="name-hint" className={`text-xs ${badName ? 'text-status-err' : 'text-muted-foreground'}`}>
          {badName ?? (taken ? 'A stack with this name already exists.' : `The directory /${name || '<name>'} under the stacks root.`)}
        </p>
      </div>
      <section aria-labelledby="new-compose" className="flex min-w-0 flex-col gap-1">
        <h2 id="new-compose" className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          Compose file <span className="font-mono normal-case">compose.yaml</span>
        </h2>
        <Suspense fallback={<Loading />}>
          <ComposeEditor value={compose} onChange={setCompose} label="compose.yaml" problems={yamlErrors} />
        </Suspense>
      </section>
      <section aria-labelledby="new-env" className="flex min-w-0 flex-col gap-1">
        <h2 id="new-env" className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          Environment <span className="font-mono normal-case">.env</span> (optional)
        </h2>
        <Suspense fallback={<Loading />}>
          <EnvEditor value={env} onChange={setEnv} />
        </Suspense>
      </section>
      <div className="flex flex-wrap gap-1">
        <Button variant="outline" size="sm" disabled={!canCreate} onClick={() => create(false)} data-action="create">
          <PlusIcon />
          Create
        </Button>
        <Button size="sm" disabled={!canCreate} onClick={() => create(true)} data-action="create-deploy">
          <RocketLaunchIcon />
          Create &amp; deploy
        </Button>
      </div>
      <PreviewDialog />
    </>
  )
}

export default function CreateStack() {
  const me = useSession((s) => s.me)
  const load = useStacks((s) => s.load)
  useEffect(() => {
    load()
  }, [load])

  let body: ReactNode
  if (me?.role !== 'admin') body = <p role="status" className="text-xs text-muted-foreground">Only admins can create stacks.</p>
  else if (!me.features.edit) body = <p role="status" className="text-xs text-muted-foreground">Editing is turned off on this server.</p>
  else body = <Form />

  return (
    <section data-page="new-stack" aria-labelledby="new-title" className="flex min-w-0 flex-col gap-2 p-2">
      <header className="flex min-w-0 flex-col gap-1">
        <Link to="/" className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-primary">
          <ArrowLeftIcon aria-hidden className="size-3" /> Stacks
        </Link>
        <h1 id="new-title" className="font-display text-sm font-semibold tracking-wider">
          New stack
        </h1>
      </header>
      {body}
    </section>
  )
}
