/**
 * Bindings (spec 055 US1, T052): repositories bound to a project, the jobs their
 * `.keel/jobs.yaml` defines at the last synced commit, and the workspace's credentials.
 * Every change is previewed and confirmed (./parts.tsx); credential values are sent once and
 * never shown back, only their fingerprints.
 */
import { PlusIcon, XIcon } from '@phosphor-icons/react'
import { Button, Input } from '@companion/ui'
import {
  DataTable,
  EmptyState,
  emptyVariant,
  FIELD,
  Field as FormField,
  H2,
  LABEL,
  Link,
  LINK,
  LoadFailed,
  LoadState,
  Page,
  SectionIntro,
  Select,
  STATUS,
  useReveal,
} from '@companion/ui/kit'
import { useId, useRef, useState, type SyntheticEvent } from 'react'

import { useSession } from '@/app/session-store'
import { canAdminister, canOperate, useWorkspaces } from '@/features/hosts/workspaces'
import { TermTip } from '@/help/TermTip'
import type { BindingInfo, CredentialInfo, CredentialKind, JobSummary, Transport } from '@/protocol/wire'
import { PageHeader } from '@/ui/PageHeader'

import { ErrorLine, Field, PreviewDialog, WorkspacePicker } from './parts'
import { short, useRepos } from './store'

function tone(status: string): string {
  return status === 'active' ? 'text-status-ok' : status === 'disabled' ? 'text-muted-foreground' : 'text-status-warn'
}

function Status({ status, ...rest }: { status: string; [data: `data-${string}`]: boolean }) {
  return (
    <span {...rest} className={`${STATUS} ${tone(status)}`}>
      {status}
    </span>
  )
}

const KINDS: { kind: CredentialKind; label: string }[] = [
  { kind: 'git_https_token', label: 'Git HTTPS token' },
  { kind: 'git_ssh_key', label: 'Git SSH key' },
  { kind: 'vault_password', label: 'Vault password' },
  { kind: 'env', label: 'Environment' },
  { kind: 'secret_vars', label: 'Secret vars' },
]

/**
 * The project a new binding lands in, picked from the current workspace's projects (ListWorkspaces).
 * One project is picked for you; none sends you to Teams to create one.
 */
function ProjectField({ id, value, onChange }: { id: string; value: string; onChange: (project: string) => void }) {
  const workspace = useRepos((s) => s.workspace)
  const { all } = useWorkspaces()
  const projects = all === null ? null : (all.find((w) => w.id === workspace)?.projects ?? [])
  return (
    <Field label="Project" id={id}>
      {projects === null ? (
        <p role="status" className="text-xs text-muted-foreground">
          Loading projects…
        </p>
      ) : projects.length === 0 ? (
        <p data-no-projects className="text-xs text-muted-foreground">
          This workspace has no projects yet.{' '}
          <Link to="/teams" className="text-accent-neon underline underline-offset-2">
            Create one in Teams
          </Link>
        </p>
      ) : (
        <Select id={id} value={value} onChange={(e) => onChange(e.target.value)} required>
          <option value="">Choose a project…</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name} ({p.slug})
            </option>
          ))}
        </Select>
      )}
    </Field>
  )
}

export function NewBinding() {
  const propose = useRepos((s) => s.propose)
  const credentials = useRepos((s) => s.credentials)
  const workspace = useRepos((s) => s.workspace)
  const { all } = useWorkspaces()
  const projects = all?.find((w) => w.id === workspace)?.projects ?? []
  const [picked, setProject] = useState('')
  // A workspace with one project needs no choice.
  const project = picked !== '' ? picked : projects.length === 1 ? (projects[0]?.id ?? '') : ''
  const [url, setUrl] = useState('')
  const [branch, setBranch] = useState('main')
  const [transport, setTransport] = useState<Transport>('https')
  const [credential, setCredential] = useState('')
  const [knownHost, setKnownHost] = useState('')
  const [poll, setPoll] = useState('')
  const id = useId()

  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    const secs = poll.trim() === '' ? null : Number(poll)
    propose({
      type: 'PreviewBindingCreate',
      data: {
        project,
        url: url.trim(),
        branch: branch.trim(),
        transport,
        credential: credential || null,
        known_host: transport === 'ssh' && knownHost.trim() ? knownHost.trim() : null,
        poll_interval: secs !== null && Number.isFinite(secs) ? secs : null,
        allow_force_push: false,
        signed_mode: false,
        override_quorum: 1,
      },
    })
  }

  const usable = credentials.filter((c) =>
    transport === 'ssh' ? c.kind === 'git_ssh_key' : c.kind === 'git_https_token',
  )
  return (
    <form
      aria-label="New binding"
      onSubmit={submit}
      className="grid min-w-0 gap-3 border border-border bg-muted/20 p-3 sm:grid-cols-2"
    >
      <div className="flex min-w-0 items-center gap-1.5 sm:col-span-2">
        <h2 className={H2}>New binding</h2>
        <TermTip term="binding" />
      </div>
      <ProjectField id={`${id}-project`} value={project} onChange={setProject} />
      <Field label="Repository URL" id={`${id}-url`}>
        <Input id={`${id}-url`} value={url} onChange={(e) => setUrl(e.target.value)} required />
      </Field>
      <Field label="Branch" id={`${id}-branch`}>
        <Input id={`${id}-branch`} value={branch} onChange={(e) => setBranch(e.target.value)} required />
      </Field>
      <Field label="Transport" id={`${id}-transport`}>
        <select
          id={`${id}-transport`}
          className={FIELD}
          value={transport}
          onChange={(e) => setTransport(e.target.value as Transport)}
        >
          <option value="https">HTTPS</option>
          <option value="ssh">SSH</option>
        </select>
      </Field>
      <Field label="Credential" id={`${id}-credential`}>
        <select id={`${id}-credential`} className={FIELD} value={credential} onChange={(e) => setCredential(e.target.value)}>
          <option value="">None (public)</option>
          {usable.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
      </Field>
      {transport === 'ssh' && (
        <Field label="Known host line" id={`${id}-known`}>
          <Input id={`${id}-known`} value={knownHost} onChange={(e) => setKnownHost(e.target.value)} required />
        </Field>
      )}
      <FormField label="Poll every (seconds)" htmlFor={`${id}-poll`} hint="Leave blank to sync only when a webhook arrives.">
        <Input id={`${id}-poll`} inputMode="numeric" value={poll} onChange={(e) => setPoll(e.target.value)} />
      </FormField>
      <div className="flex items-start sm:col-span-2">
        <Button type="submit" disabled={project === ''}>
          Preview binding
        </Button>
      </div>
    </form>
  )
}

/** One binding. Syncing needs Operate; every other change, and granting credentials, Administer. */
function Detail({ binding, canSync, canAct }: { binding: BindingInfo; canSync: boolean; canAct: boolean }) {
  const detail = useRepos((s) => s.detail)
  const credentials = useRepos((s) => s.credentials)
  const propose = useRepos((s) => s.propose)
  const sync = useRepos((s) => s.sync)
  const open = useRepos((s) => s.open)
  const [grant, setGrant] = useState('')
  const id = useId()
  const jobs = detail?.binding.id === binding.id ? detail.jobs : []
  const granted = credentials.filter((c) => c.grants.some((g) => g.binding === binding.id))

  const reveal = useReveal<HTMLElement>(binding.id)

  return (
    <section ref={reveal} aria-label={`Binding ${binding.url}`} className="flex min-w-0 scroll-mt-3 flex-col border border-border">
      <div className="flex min-h-10 min-w-0 flex-wrap items-center gap-2 border-b border-border bg-muted/20 px-3 py-1.5">
        <h2 className="min-w-0 font-mono text-xs break-all">
          {binding.url} @ {binding.branch}
        </h2>
        <Status data-binding-status status={binding.status} />
        <div className="ml-auto flex flex-wrap gap-1">
          {canSync && (
            <Button size="xs" variant="outline" onClick={() => sync(binding.id)}>
              Sync now
            </Button>
          )}
          {canAct && (
            <>
              <Button
                size="xs"
                variant="outline"
                onClick={() =>
                  propose({
                    type: binding.status === 'active' ? 'PreviewBindingDisable' : 'PreviewBindingEnable',
                    data: { binding: binding.id },
                  })
                }
              >
                {binding.status === 'active' ? 'Disable' : 'Enable'}
              </Button>
              <Button
                size="xs"
                variant="destructive"
                onClick={() => propose({ type: 'PreviewBindingRemove', data: { binding: binding.id } })}
              >
                Remove
              </Button>
            </>
          )}
          <Button size="xs" variant="ghost" onClick={() => open(null)}>
            <XIcon aria-hidden />
            Close
          </Button>
        </div>
      </div>
      <div className="flex min-w-0 flex-col gap-4 p-3">
        <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-xs">
          <dt className="text-muted-foreground">Last commit</dt>
          <dd className="font-mono">{short(binding.last_commit)}</dd>
          <dt className="text-muted-foreground">Synced</dt>
          <dd>{binding.last_synced_at ?? 'never'}</dd>
          <dt className="text-muted-foreground">Polling</dt>
          <dd>{binding.poll_interval === null ? 'webhook only' : `every ${String(binding.poll_interval)}s`}</dd>
          {binding.last_sync_error && (
            <>
              <dt className="text-muted-foreground">Error</dt>
              <dd role="status" data-sync-error className="break-words text-status-err">
                {binding.last_sync_error}
              </dd>
            </>
          )}
        </dl>
        {binding.manifest_problems.length > 0 && (
          <ul aria-label="Manifest problems" className="flex min-w-0 flex-col gap-1 border border-status-err/40 p-2 text-xs text-status-err">
            {binding.manifest_problems.map((p, i) => (
              <li key={i} className="break-words">
                <span className="font-mono break-all">{p.at}</span>: {p.message}
              </li>
            ))}
          </ul>
        )}
        <div className="flex min-w-0 flex-col gap-2">
          <h3 className={LABEL}>Jobs at {short(binding.last_commit)}</h3>
          <DataTable<JobSummary>
            label="Jobs"
            rows={jobs}
            rowKey={(j) => j.name}
            columns={[
              { key: 'job', header: 'Job', cell: (j) => <span className="font-mono">{j.name}</span> },
              { key: 'playbook', header: 'Playbook', cell: (j) => <span className="font-mono break-all">{j.playbook}</span> },
              { key: 'profiles', header: 'Profiles', tip: <TermTip term="profile" />, cell: (j) => j.profiles.join(', ') },
              { key: 'inventory', header: 'Inventory', cell: (j) => j.inventory },
              {
                key: 'state',
                header: 'State',
                cell: (j) =>
                  j.blocked ? <span className="text-status-err">blocked: {j.blocked}</span> : <span className="text-status-ok">ready</span>,
              },
            ]}
            empty="No jobs at this commit. Jobs come from the repository's .keel/jobs.yaml."
          />
        </div>
        {canAct && (
          <div className="flex min-w-0 flex-wrap items-end gap-2">
            <Field label="Grant a credential" id={`${id}-grant`}>
              <select id={`${id}-grant`} className={FIELD} value={grant} onChange={(e) => setGrant(e.target.value)}>
                <option value="">Choose…</option>
                {credentials.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
            </Field>
            <Button
              size="sm"
              variant="outline"
              disabled={!grant}
              onClick={() =>
                propose({ type: 'PreviewCredentialGrant', data: { credential: grant, binding: binding.id, profiles: [] } })
              }
            >
              Grant
            </Button>
            {granted.map((c) => (
              <Button
                key={c.id}
                size="xs"
                variant="ghost"
                onClick={() => propose({ type: 'PreviewCredentialRevoke', data: { credential: c.id, binding: binding.id } })}
              >
                Revoke {c.name}
              </Button>
            ))}
          </div>
        )}
      </div>
    </section>
  )
}

/** No credentials in the workspace yet (spec 063 FR-011, FR-012). */
function NoCredentials({ canAct, onAdd }: { canAct: boolean; onAdd: () => void }) {
  return (
    <EmptyState
      page="credentials"
      title={canAct ? 'Add a credential' : 'No credentials yet'}
      reason="A private repository needs a token or SSH key before Keel can fetch it."
      steps={['Add a token or SSH key.', 'Grant it to the binding that needs it.']}
      action={
        canAct ? (
          // Outline: the bindings list above holds the page's one primary action.
          <Button size="sm" variant="outline" onClick={onAdd}>
            <PlusIcon aria-hidden />
            Add a credential
          </Button>
        ) : undefined
      }
      whoCan="A workspace admin"
      verb="add credentials"
      guide={{ guide: 'bindings', anchor: 'credentials' }}
    />
  )
}

function Credentials({ canAct }: { canAct: boolean }) {
  const workspace = useRepos((s) => s.workspace)
  const credentials = useRepos((s) => s.credentials)
  const loaded = useRepos((s) => s.loaded.credentials)
  const failed = useRepos((s) => s.failed.credentials)
  const load = useRepos((s) => s.load)
  const propose = useRepos((s) => s.propose)
  const nameRef = useRef<HTMLInputElement>(null)
  const [name, setName] = useState('')
  const [kind, setKind] = useState<CredentialKind>('git_https_token')
  const [value, setValue] = useState('')
  const id = useId()

  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    if (!workspace) return
    propose({ type: 'PreviewCredentialCreate', data: { workspace, name: name.trim(), kind, value } })
    // The value leaves the form as soon as it is sent.
    setValue('')
  }

  return (
    <section aria-labelledby={`${id}-title`} className="flex min-w-0 flex-col gap-3">
      <div className="flex min-w-0 flex-col gap-0.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <h2 id={`${id}-title`} className={H2}>
            Credentials
          </h2>
          <TermTip term="credential" />
        </div>
        <SectionIntro guide={{ guide: 'bindings', anchor: 'credentials' }}>
          Secrets a binding uses to fetch or run.
        </SectionIntro>
      </div>
      {workspace && failed ? (
        <LoadFailed
          what="credentials"
          refusal={failed}
          onRetry={() => {
            load(workspace)
          }}
        />
      ) : loaded && credentials.length === 0 ? (
        <NoCredentials
          canAct={canAct}
          onAdd={() => {
            nameRef.current?.focus()
          }}
        />
      ) : (
        <DataTable<CredentialInfo>
          label="Credentials"
          rows={credentials}
          rowKey={(c) => c.id}
          columns={[
            { key: 'name', header: 'Name', cell: (c) => <span className="font-mono">{c.name}</span> },
            { key: 'kind', header: 'Kind', cell: (c) => KINDS.find((k) => k.kind === c.kind)?.label ?? c.kind },
            { key: 'fp', header: 'Fingerprint', tip: <TermTip term="fingerprint" />, className: 'w-[40%]', cell: (c) => <span className="font-mono break-all">{c.fingerprint}</span> },
            { key: 'grants', header: 'Granted to', cell: (c) => <span className="tabular-nums">{c.grants.length}</span> },
          ]}
          empty={<LoadState loading label="Loading credentials…" />}
        />
      )}
      {canAct && (
        <form aria-label="New credential" onSubmit={submit} className="flex min-w-0 flex-wrap items-end gap-2 border border-border bg-muted/20 p-3">
          <Field label="Name" id={`${id}-name`}>
            <Input id={`${id}-name`} ref={nameRef} value={name} onChange={(e) => setName(e.target.value)} required />
          </Field>
          <Field label="Kind" id={`${id}-kind`}>
            <select id={`${id}-kind`} className={FIELD} value={kind} onChange={(e) => setKind(e.target.value as CredentialKind)}>
              {KINDS.map((k) => (
                <option key={k.kind} value={k.kind}>
                  {k.label}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Value" id={`${id}-value`}>
            <Input
              id={`${id}-value`}
              type="password"
              autoComplete="off"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              required
            />
          </Field>
          <Button type="submit" size="sm">
            Preview credential
          </Button>
        </form>
      )}
    </section>
  )
}

export default function BindingsPage() {
  const bindings = useRepos((s) => s.bindings)
  const detail = useRepos((s) => s.detail)
  const workspace = useRepos((s) => s.workspace)
  const open = useRepos((s) => s.open)
  const loaded = useRepos((s) => s.loaded.bindings)
  const failed = useRepos((s) => s.failed.bindings)
  const load = useRepos((s) => s.load)
  const me = useSession((s) => s.me)
  const [adding, setAdding] = useState(false)
  const variant = emptyVariant({ filtered: false, canAct: workspace !== null && canAdminister(me, workspace) })
  const canAct = variant === 'guided'
  const canSync = workspace !== null && canOperate(me, workspace)
  // The guided state stands in for the table once the list has answered empty, until the form opens.
  const empty = !failed && loaded && bindings.length === 0 && !adding
  const current = detail ? (bindings.find((b) => b.id === detail.binding.id) ?? detail.binding) : null

  return (
    <Page name="bindings" labelledBy="bindings-title">
      <PageHeader
        id="bindings-title"
        title="Bindings"
        tip={<TermTip term="binding" />}
        description="Branches Keel runs from, their jobs and their credentials."
        guide={{ guide: 'bindings' }}
        actions={
          workspace &&
          canAct && (
            <Button size="sm" variant={adding ? 'ghost' : empty ? 'outline' : 'default'} aria-expanded={adding} onClick={() => setAdding((a) => !a)}>
              {adding ? <XIcon aria-hidden /> : <PlusIcon aria-hidden />}
              {adding ? 'Close the form' : 'New binding'}
            </Button>
          )
        }
      >
        <WorkspacePicker />
      </PageHeader>
      <ErrorLine />
      {adding && <NewBinding />}
      {workspace && empty && (
        <EmptyState
          page="bindings"
          title={canAct ? 'Bind a repository' : 'No repository is bound yet'}
          reason="A binding says which branch Keel runs, and where."
          steps={[
            'Add a credential if the repository is private.',
            'Bind a branch to this workspace.',
            'Push to the branch.',
          ]}
          action={
            canAct ? (
              <Button
                size="sm"
                onClick={() => {
                  setAdding(true)
                }}
              >
                <PlusIcon aria-hidden />
                Bind a repository
              </Button>
            ) : undefined
          }
          whoCan="A workspace admin"
          verb="bind repositories"
          guide={{ guide: 'bindings', anchor: 'bind-a-repository' }}
        />
      )}
      {workspace && failed && (
        <LoadFailed
          what="bindings"
          refusal={failed}
          onRetry={() => {
            load(workspace)
          }}
        />
      )}
      {workspace && !failed && !empty && (
        <DataTable<BindingInfo>
          label="Bindings"
          rows={bindings}
          rowKey={(b) => b.id}
          selected={(b) => b.id === current?.id}
          columns={[
            {
              key: 'url',
              header: 'Repository',
              className: 'w-[36%]',
              cell: (b) => (
                <button type="button" className={`${LINK} font-mono break-all`} onClick={() => open(b.id)}>
                  {b.url}
                </button>
              ),
            },
            { key: 'branch', header: 'Branch', cell: (b) => <span className="font-mono">{b.branch}</span> },
            { key: 'status', header: 'Status', cell: (b) => <Status status={b.status} /> },
            { key: 'commit', header: 'Commit', cell: (b) => <span className="font-mono">{short(b.last_commit)}</span> },
            { key: 'error', header: 'Error', cell: (b) => <span className="text-status-err">{b.last_sync_error ?? ''}</span> },
          ]}
          empty={loaded ? 'No repository is bound yet.' : <LoadState loading label="Loading bindings…" />}
        />
      )}
      {current && <Detail binding={current} canSync={canSync} canAct={canAct} />}
      {workspace && <Credentials canAct={canAct} />}
      <PreviewDialog />
    </Page>
  )
}
