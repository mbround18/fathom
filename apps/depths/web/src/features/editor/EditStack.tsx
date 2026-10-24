/**
 * `/stacks/:name/edit` (US3, T061): edit the compose file and `.env`, then save through a server
 * preview (`PreviewSave` with the `base_hash` loaded), alone or with a deploy (FR-020).
 *
 * - YAML errors are found on every keystroke and block saving; the server's compose errors from
 *   the last attempt (`ValidationFailed`) are marked at their lines (FR-017).
 * - The diff tab shows exactly what changed against what was loaded (FR-018); the preview shows
 *   the server's own diff before anything is written.
 * - A save refused with `Conflict` shows the other change, writes nothing, and lets the person
 *   take theirs or keep their own on top (FR-019).
 * - `.env` with secret-looking keys stays masked and read-only until revealed (`RevealEnv`,
 *   audited; FR-021). Leaving `.env` untouched sends no `env`, so it is left as it is.
 * - Unsaved changes ask before leaving.
 */
import { ArrowLeftIcon, FloppyDiskIcon, RocketLaunchIcon } from '@phosphor-icons/react'
import { YamlDiffView } from '@companion/terminal'
import { Button } from '@companion/ui'
import { lazy, Suspense, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'

import { Link } from '@/app/Link'
import { navigate, type Params } from '@/app/router'
import { useSession } from '@/app/session-store'
import type { EnvText, StackDetail } from '@/protocol/generated'
import { DENY_TEXT } from '@/features/stacks/argv'
import { editBlockedReason, maskEnv } from '@/features/stacks/model'
import { useOps } from '@/features/stacks/ops-store'
import { PreviewDialog } from '@/features/stacks/PreviewDialog'
import { useStacks } from '@/features/stacks/store'

import { problemText, serverProblems, yamlProblems } from './problems'
import { useReveal } from './reveal-store'
import { useUnsavedGuard } from './useUnsavedGuard'

const ComposeEditor = lazy(() => import('./ComposeEditor'))
const EnvEditor = lazy(() => import('./EnvEditor'))
const CodeView = lazy(() => import('./CodeView'))

/** How `.env` can be edited as loaded. */
type EnvBase =
  | { mode: 'edit'; text: string }
  | { mode: 'locked'; masked: string }
  | { mode: 'unavailable' }

function envBase(env: EnvText | null, hasEnv: boolean): EnvBase {
  if (env === null) return hasEnv ? { mode: 'unavailable' } : { mode: 'edit', text: '' }
  // Plain text with no secret-looking key is safe to show; anything else waits for a reveal.
  if (env.kind === 'plain' && maskEnv(env.text) === env.text) return { mode: 'edit', text: env.text }
  return { mode: 'locked', masked: env.kind === 'masked' ? env.text : maskEnv(env.text) }
}

interface Base {
  hash: string
  compose: string
  env: EnvBase
}

function Loading({ what }: { what: string }) {
  return (
    <p role="status" data-pane-state="loading" className="border border-border p-2 text-xs text-muted-foreground">
      Loading {what}…
    </p>
  )
}

function SectionTitle({ id, children }: { id: string; children: ReactNode }) {
  return (
    <h2 id={id} className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
      {children}
    </h2>
  )
}

function Editor({ detail }: { detail: StackDetail }) {
  const name = detail.stack.name
  const [base, setBase] = useState<Base>(() => ({
    hash: detail.content_hash,
    compose: detail.compose_text ?? '',
    env: envBase(detail.env, detail.stack.has_env),
  }))
  const [compose, setCompose] = useState(base.compose)
  /** `null`: `.env` untouched. */
  const [envDraft, setEnvDraft] = useState<string | null>(null)
  const [tab, setTab] = useState<'edit' | 'diff'>('edit')

  const revealed = useReveal((s) => s.env[name])
  const reveal = useReveal((s) => s.reveal)
  const flow = useOps((s) => s.flow)
  const run = useOps((s) => s.ops[name])
  const requestPreview = useOps((s) => s.requestPreview)
  const dismiss = useOps((s) => s.dismiss)

  const envBaseText = base.env.mode === 'edit' ? base.env.text : revealed?.state === 'ready' ? revealed.text : null
  const envText = envDraft ?? envBaseText
  const envChanged = envDraft !== null && envDraft !== envBaseText
  const dirty = compose !== base.compose || envChanged

  const mine = flow?.request.kind === 'save' && flow.request.draft.stack === name ? flow : null
  const refusal = mine?.refusal ?? null
  const conflict = refusal?.kind === 'conflict' ? refusal : null
  const yamlErrors = useMemo(() => yamlProblems(compose), [compose])
  const validation = refusal?.kind === 'validation' ? refusal.errors : null
  const serverErrors = useMemo(() => (validation ? serverProblems(validation, 'compose') : []), [validation])
  const envErrors = useMemo(() => (validation ? serverProblems(validation, 'env') : []), [validation])
  const problems = useMemo(() => [...yamlErrors, ...serverErrors], [yamlErrors, serverErrors])

  const { release } = useUnsavedGuard(dirty)

  // A save whose operation started: its output is on the stack page.
  const awaiting = useRef<{ before: string | null } | null>(null)
  useEffect(() => {
    const w = awaiting.current
    if (!w || !run || run.kind !== 'save' || run.operationId === w.before) return
    awaiting.current = null
    release()
    navigate(`/stacks/${encodeURIComponent(name)}`)
  }, [run, name, release])

  // Leaving: drop a save flow still open for this stack, and the revealed text.
  useEffect(
    () => () => {
      const f = useOps.getState().flow
      if (f?.request.kind === 'save' && f.request.draft.stack === name && f.phase !== 'confirming') useOps.getState().dismiss()
      useReveal.getState().forget(name)
    },
    [name],
  )

  const save = (deploy: boolean): void => {
    awaiting.current = { before: run?.operationId ?? null }
    requestPreview({
      kind: 'save',
      draft: { stack: name, base_hash: base.hash, compose, ...(envChanged && envText !== null ? { env: envText } : {}) },
      deploy,
    })
  }

  const takeTheirs = (): void => {
    if (!conflict) return
    const env = envBase(conflict.currentEnv, conflict.currentEnv !== null)
    setBase({ hash: conflict.currentHash, compose: conflict.currentText, env })
    setCompose(conflict.currentText)
    setEnvDraft(null)
    useReveal.getState().forget(name)
    dismiss()
  }
  const keepMine = (): void => {
    if (!conflict) return
    setBase((b) => ({ ...b, hash: conflict.currentHash }))
    dismiss()
  }

  const asking = mine !== null && (mine.phase === 'previewing' || mine.phase === 'confirming')
  const canSave = dirty && yamlErrors.length === 0 && !asking
  const changedOnDisk = detail.content_hash !== base.hash && conflict === null

  return (
    <>
      <div className="flex min-w-0 flex-wrap items-center gap-1" role="toolbar" aria-label="Editor">
        <div className="flex border border-border" role="group" aria-label="View">
          <Button variant={tab === 'edit' ? 'secondary' : 'ghost'} size="xs" aria-pressed={tab === 'edit'} onClick={() => setTab('edit')}>
            Edit
          </Button>
          <Button variant={tab === 'diff' ? 'secondary' : 'ghost'} size="xs" aria-pressed={tab === 'diff'} onClick={() => setTab('diff')}>
            Diff
          </Button>
        </div>
        <span className="flex-1" />
        <span data-field="dirty" className="text-xs text-muted-foreground">
          {dirty ? 'Unsaved changes' : 'No changes'}
        </span>
        <Button variant="outline" size="sm" disabled={!canSave} onClick={() => save(false)} data-action="save">
          <FloppyDiskIcon />
          Save
        </Button>
        <Button size="sm" disabled={!canSave} onClick={() => save(true)} data-action="save-deploy">
          <RocketLaunchIcon />
          Save &amp; deploy
        </Button>
      </div>
      {yamlErrors.length > 0 && (
        <p role="alert" data-field="save-blocked" className="text-xs text-status-err">
          Fix the YAML errors to save.
        </p>
      )}
      {changedOnDisk && (
        <p role="status" data-field="changed-on-disk" className="border border-status-warn/60 bg-status-warn/10 p-1.5 text-xs text-status-warn">
          The files changed on disk since you opened them. Saving will show you their change first.
        </p>
      )}
      {serverErrors.length + envErrors.length > 0 && (
        <div role="alert" data-field="server-errors" className="text-xs text-status-err">
          <p>The server refused the {serverErrors.length > 0 ? 'compose file' : '.env file'}. Nothing was written.</p>
          {/* A masked .env has no editor to mark, so its errors are listed here. */}
          {envText === null && envErrors.length > 0 && (
            <ul data-field="env-errors" className="list-inside list-disc">
              {envErrors.map((p, i) => (
                <li key={i}>{problemText(p)}</li>
              ))}
            </ul>
          )}
        </div>
      )}
      {conflict && (
        <section data-conflict aria-labelledby="conflict-title" className="flex min-w-0 flex-col gap-1 border border-status-warn/60 p-2">
          <h2 id="conflict-title" className="text-xs font-semibold text-status-warn">
            Someone else saved first. Nothing was written.
          </h2>
          <p className="text-xs text-muted-foreground">Their change to {detail.stack.compose_file}, against what you opened:</p>
          <div data-their-change className="min-w-0 overflow-x-auto border border-border">
            <YamlDiffView currentManifest={base.compose} targetManifest={conflict.currentText} />
          </div>
          <div className="flex flex-wrap gap-1">
            <Button variant="outline" size="sm" onClick={takeTheirs} data-action="take-theirs">
              Use their version
            </Button>
            <Button variant="secondary" size="sm" onClick={keepMine} data-action="keep-mine">
              Keep mine on top of theirs
            </Button>
          </div>
        </section>
      )}

      {tab === 'diff' ? (
        <section data-editor-diff aria-label="Changes" className="flex min-w-0 flex-col gap-2">
          <SectionTitle id="diff-compose">{detail.stack.compose_file}</SectionTitle>
          <div className="min-w-0 overflow-x-auto border border-border">
            <YamlDiffView currentManifest={base.compose} targetManifest={compose} />
          </div>
          {envChanged && envBaseText !== null && envText !== null && (
            <>
              <SectionTitle id="diff-env">.env</SectionTitle>
              <div className="min-w-0 overflow-x-auto border border-border">
                <YamlDiffView currentManifest={envBaseText} targetManifest={envText} />
              </div>
            </>
          )}
        </section>
      ) : (
        <>
          <section aria-labelledby="edit-compose" className="flex min-w-0 flex-col gap-1">
            <SectionTitle id="edit-compose">
              Compose file <span className="font-mono normal-case">{detail.stack.compose_file}</span>
            </SectionTitle>
            <Suspense fallback={<Loading what="editor" />}>
              <ComposeEditor value={compose} onChange={setCompose} label={detail.stack.compose_file} problems={problems} />
            </Suspense>
          </section>
          <section aria-labelledby="edit-env" className="flex min-w-0 flex-col gap-1">
            <SectionTitle id="edit-env">
              Environment <span className="font-mono normal-case">.env</span>
            </SectionTitle>
            {base.env.mode === 'unavailable' ? (
              <p className="text-xs text-muted-foreground">Too large to edit here.</p>
            ) : envText !== null ? (
              <Suspense fallback={<Loading what="editor" />}>
                <EnvEditor value={envText} onChange={setEnvDraft} problems={envErrors} />
              </Suspense>
            ) : (
              <>
                <p className="text-xs text-muted-foreground">
                  Values of keys that look secret are masked. Reveal them to edit; revealing is recorded in the audit log.
                </p>
                <Suspense fallback={<Loading what="viewer" />}>
                  <CodeView text={base.env.mode === 'locked' ? base.env.masked : ''} language="env" label=".env" />
                </Suspense>
                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    data-action="reveal-env"
                    disabled={revealed?.state === 'asking'}
                    onClick={() => reveal(name)}
                  >
                    {revealed?.state === 'asking' ? 'Revealing…' : 'Reveal to edit'}
                  </Button>
                  {revealed?.state === 'denied' && (
                    <span role="alert" className="text-xs text-status-err">
                      {revealed.reason === null ? 'The connection dropped. Try again.' : DENY_TEXT[revealed.reason]}
                    </span>
                  )}
                </div>
              </>
            )}
          </section>
        </>
      )}
      <PreviewDialog />
    </>
  )
}

export default function EditStack({ params }: { params: Params }) {
  const name = params.name ?? ''
  const entry = useStacks((s) => s.details[name])
  const openDetail = useStacks((s) => s.openDetail)
  const closeDetail = useStacks((s) => s.closeDetail)
  const load = useStacks((s) => s.load)
  const editOn = useSession((s) => s.me?.features.edit ?? false)

  useEffect(() => {
    load()
    openDetail(name)
    return () => closeDetail(name)
  }, [name, load, openDetail, closeDetail])

  const back = (
    <Link
      to={`/stacks/${encodeURIComponent(name)}`}
      className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-primary"
    >
      <ArrowLeftIcon aria-hidden className="size-3" /> {name}
    </Link>
  )

  let body: ReactNode
  if (!editOn) body = <p role="status" className="text-xs text-muted-foreground">Editing is turned off on this server.</p>
  else if (entry?.state === 'denied') body = <p role="status" className="text-xs text-muted-foreground">This stack is not available to you.</p>
  else if (entry?.state !== 'ready') body = <Loading what={name} />
  else {
    const reason = editBlockedReason(entry.detail)
    body =
      reason !== null ? (
        <p role="status" data-field="not-editable" className="text-xs text-muted-foreground">
          {reason}
        </p>
      ) : (
        <Editor key={name} detail={entry.detail} />
      )
  }

  return (
    <section data-page="edit" aria-labelledby="edit-title" className="flex min-w-0 flex-col gap-2 p-2">
      <header className="flex min-w-0 flex-col gap-1">
        {back}
        <h1 id="edit-title" className="min-w-0 truncate font-display text-sm font-semibold tracking-wider">
          Edit {name}
        </h1>
      </header>
      {body}
    </section>
  )
}
