/**
 * Settings → Workspace (spec 066 T034, contracts/ui.md "Settings → Workspace"): the current
 * workspace's repository, its forge connection and the danger zone, under the workspace's name.
 *
 * With no primary repository the guided form binds one (`PreviewWorkspaceRepository`, onto the
 * workspace's `default` project); with one, a summary says where it stands and offers Sync now,
 * Open binding and a choice of another primary. The forge connection is saved, tested and removed
 * here. Every change previews and confirms through the shell's `ChangeDialog`, and the tab loads
 * again once one is confirmed.
 */
import { Button, Input } from '@companion/ui'
import { Field, FIELD, H2, LoadFailed, LoadState, navigate } from '@companion/ui/kit'
import { useCallback, useId, useState, type ReactNode, type SyntheticEvent } from 'react'

import { CurrentWorkspace } from '@/app/WorkspacePicker'
import { useCurrentWorkspace } from '@/app/workspace-store'
import { short, useRepos } from '@/features/bindings/store'
import { useGovernance } from '@/features/teams/store'
import type { BindingInfo, CredentialKind, Event, ForgeProvider, Transport, WorkspaceInfo } from '@/protocol/wire'

import { defaultApiBase, useForge } from './forge-store'

const NEW = '__new__'

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} data-section={id} className="flex min-w-0 flex-col gap-3 border border-border p-3">
      <h2 id={id} className={H2}>
        {title}
      </h2>
      {children}
    </section>
  )
}

/** Reloads the workspace's lists once a change is confirmed. */
function reloadOnConfirm(workspace: string) {
  return (event: Event) => {
    if (event.type !== 'Confirmed') return
    useRepos.getState().load(workspace)
    useForge.getState().load(workspace)
  }
}

/** A credential of `kind`, or "New credential…", which opens the form to add one. */
function CredentialSelect({
  id,
  workspace,
  kind,
  value,
  onChange,
  optional,
}: {
  id: string
  workspace: string
  kind: CredentialKind
  value: string
  onChange: (id: string) => void
  optional: boolean
}) {
  const credentials = useRepos((s) => s.credentials).filter((c) => c.kind === kind)
  const propose = useGovernance((s) => s.propose)
  const [adding, setAdding] = useState(false)
  const [name, setName] = useState('')
  const [secret, setSecret] = useState('')

  const add = () => {
    if (name.trim() === '' || secret === '') return
    propose('New credential', { type: 'PreviewCredentialCreate', data: { workspace, name: name.trim(), kind, value: secret } }, reloadOnConfirm(workspace))
    // The value leaves the form as soon as it is sent.
    setSecret('')
    setName('')
    setAdding(false)
  }

  return (
    <div className="flex min-w-0 flex-col gap-2">
      <select
        id={id}
        className={FIELD}
        value={adding ? NEW : value}
        onChange={(e) => {
          if (e.target.value === NEW) setAdding(true)
          else {
            setAdding(false)
            onChange(e.target.value)
          }
        }}
      >
        <option value="">{optional ? 'None (public)' : 'Choose a credential'}</option>
        {credentials.map((c) => (
          <option key={c.id} value={c.id}>
            {c.name}
          </option>
        ))}
        <option value={NEW}>New credential…</option>
      </select>
      {adding && (
        <div data-new-credential className="flex min-w-0 flex-col gap-2 border border-border bg-muted/20 p-2">
          <Field label="Credential name" htmlFor={`${id}-name`}>
            <Input id={`${id}-name`} value={name} autoComplete="off" onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label={kind === 'git_ssh_key' ? 'Private key' : 'Token'} htmlFor={`${id}-secret`} hint="Stored encrypted; never shown again.">
            <Input id={`${id}-secret`} type="password" value={secret} autoComplete="off" onChange={(e) => setSecret(e.target.value)} />
          </Field>
          <div className="flex gap-2">
            <Button type="button" size="sm" variant="outline" disabled={name.trim() === '' || secret === ''} onClick={add}>
              Preview credential
            </Button>
            <Button type="button" size="sm" variant="ghost" onClick={() => setAdding(false)}>
              Cancel
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

/** No primary repository yet: the guided form. */
function RepositoryForm({ workspace }: { workspace: WorkspaceInfo }) {
  const propose = useGovernance((s) => s.propose)
  const [url, setUrl] = useState('')
  const [branch, setBranch] = useState('main')
  const [transport, setTransport] = useState<Transport>('https')
  const [credential, setCredential] = useState('')
  const [knownHost, setKnownHost] = useState('')
  const [poll, setPoll] = useState('300')
  const id = useId()

  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    const secs = poll.trim() === '' ? null : Number(poll)
    propose(
      'Set up repository',
      {
        type: 'PreviewWorkspaceRepository',
        data: {
          workspace: workspace.id,
          spec: {
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
        },
      },
      reloadOnConfirm(workspace.id),
    )
  }

  return (
    <form aria-label="Set up repository" onSubmit={submit} className="grid min-w-0 gap-3 sm:grid-cols-2">
      <p className="text-xs text-muted-foreground sm:col-span-2">
        The Ansible repository this workspace runs from. Its roles, playbooks and inventories are synced into Keel.
      </p>
      <Field label="Repository URL" htmlFor={`${id}-url`}>
        <Input id={`${id}-url`} value={url} onChange={(e) => setUrl(e.target.value)} required placeholder="https://git.example.com/org/ansible.git" />
      </Field>
      <Field label="Branch" htmlFor={`${id}-branch`}>
        <Input id={`${id}-branch`} value={branch} onChange={(e) => setBranch(e.target.value)} required />
      </Field>
      <Field label="Transport" htmlFor={`${id}-transport`}>
        <select
          id={`${id}-transport`}
          className={FIELD}
          value={transport}
          onChange={(e) => {
            setTransport(e.target.value as Transport)
            setCredential('')
          }}
        >
          <option value="https">HTTPS</option>
          <option value="ssh">SSH</option>
        </select>
      </Field>
      <Field label="Credential" htmlFor={`${id}-credential`}>
        <CredentialSelect
          id={`${id}-credential`}
          workspace={workspace.id}
          kind={transport === 'ssh' ? 'git_ssh_key' : 'git_https_token'}
          value={credential}
          onChange={setCredential}
          optional
        />
      </Field>
      {transport === 'ssh' && (
        <Field label="Known host line" htmlFor={`${id}-known`}>
          <Input id={`${id}-known`} value={knownHost} onChange={(e) => setKnownHost(e.target.value)} required />
        </Field>
      )}
      <Field label="Poll every (seconds)" htmlFor={`${id}-poll`} hint="Leave blank to sync only on a webhook or Sync now.">
        <Input id={`${id}-poll`} inputMode="numeric" value={poll} onChange={(e) => setPoll(e.target.value)} />
      </Field>
      <div className="sm:col-span-2">
        <Button type="submit">Preview repository</Button>
      </div>
    </form>
  )
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 break-all">{children}</dd>
    </>
  )
}

/** The primary repository, where it stands, and the choice of another. */
function RepositorySummary({ workspace, primary }: { workspace: WorkspaceInfo; primary: BindingInfo }) {
  const bindings = useRepos((s) => s.bindings)
  const sync = useRepos((s) => s.sync)
  const open = useRepos((s) => s.open)
  const propose = useGovernance((s) => s.propose)
  const id = useId()

  return (
    <div data-repository={primary.id} className="flex min-w-0 flex-col gap-3">
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
        <Row label="URL">
          <span className="font-mono">{primary.url}</span>
        </Row>
        <Row label="Branch">
          <span className="font-mono">{primary.branch}</span>
        </Row>
        <Row label="Head">
          <span className="font-mono" data-head>
            {short(primary.last_commit)}
          </span>
        </Row>
        <Row label="Last sync">
          {primary.last_sync_error ? (
            <span className="text-status-err">{primary.last_sync_error}</span>
          ) : (
            (primary.last_synced_at ?? 'Not yet')
          )}
        </Row>
      </dl>
      <div className="flex flex-wrap items-end gap-2">
        <Button size="sm" variant="outline" onClick={() => sync(primary.id)}>
          Sync now
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            open(primary.id)
            navigate('/bindings')
          }}
        >
          Open binding
        </Button>
        <Field label="Change primary…" htmlFor={`${id}-primary`} className="ml-auto">
          <select
            id={`${id}-primary`}
            className={FIELD}
            value={primary.id}
            onChange={(e) =>
              propose(
                'Change primary repository',
                { type: 'PreviewWorkspacePrimary', data: { workspace: workspace.id, binding: e.target.value || null } },
                reloadOnConfirm(workspace.id),
              )
            }
          >
            {bindings.map((b) => (
              <option key={b.id} value={b.id}>
                {b.url} @ {b.branch}
              </option>
            ))}
            <option value="">None</option>
          </select>
        </Field>
      </div>
    </div>
  )
}

function Repository({ workspace }: { workspace: WorkspaceInfo }) {
  const bindings = useRepos((s) => s.bindings)
  const loaded = useRepos((s) => s.loaded.bindings)
  const failed = useRepos((s) => s.failed.bindings)
  const load = useRepos((s) => s.load)
  const primary = bindings.find((b) => b.id === workspace.primary_binding)

  if (workspace.primary_binding === null) return <RepositoryForm workspace={workspace} />
  return (
    <LoadState loading={!loaded && !primary} failed={failed} what="bindings" onRetry={() => load(workspace.id)} label="Loading repository…">
      {primary ? (
        <RepositorySummary workspace={workspace} primary={primary} />
      ) : (
        <RepositoryForm workspace={workspace} />
      )}
    </LoadState>
  )
}

function Forge({ workspace }: { workspace: WorkspaceInfo }) {
  const connection = useForge((s) => s.connection)
  const failed = useForge((s) => s.failed)
  const probing = useForge((s) => s.probing)
  const probe = useForge((s) => s.probe)
  const load = useForge((s) => s.load)
  const test = useForge((s) => s.test)
  const primaryUrl = useRepos((s) => s.bindings.find((b) => b.id === workspace.primary_binding)?.url ?? null)
  const propose = useGovernance((s) => s.propose)
  const id = useId()
  const [provider, setProvider] = useState<ForgeProvider | null>(null)
  const [apiBase, setApiBase] = useState<string | null>(null)
  const [credential, setCredential] = useState<string | null>(null)

  if (failed) return <LoadFailed what="the forge connection" refusal={failed} onRetry={() => load(workspace.id)} />
  if (connection === undefined) return <LoadState loading label="Loading forge connection…" />

  // Edits start from the saved connection.
  const p = provider ?? connection?.provider ?? 'forgejo'
  const base = apiBase ?? connection?.api_base ?? ''
  const cred = credential ?? connection?.credential ?? ''
  const fallback = defaultApiBase(p, primaryUrl)

  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    if (cred === '') return
    propose(
      connection ? 'Change forge connection' : 'Connect forge',
      { type: 'PreviewForgeConnection', data: { workspace: workspace.id, provider: p, api_base: base.trim() || null, credential: cred } },
      reloadOnConfirm(workspace.id),
    )
  }

  return (
    <form aria-label="Forge connection" onSubmit={submit} className="grid min-w-0 gap-3 sm:grid-cols-2">
      <p className="text-xs text-muted-foreground sm:col-span-2">
        Lets Keel open pull requests on the repository, for new hosts and job drafts.
      </p>
      <Field label="Provider" htmlFor={`${id}-provider`}>
        <select id={`${id}-provider`} className={FIELD} value={p} onChange={(e) => setProvider(e.target.value as ForgeProvider)}>
          <option value="forgejo">Forgejo</option>
          <option value="gitea">Gitea</option>
          <option value="github">GitHub</option>
        </select>
      </Field>
      <Field label="API base" htmlFor={`${id}-api`} hint={fallback ? `Leave blank for ${fallback}.` : undefined}>
        <Input id={`${id}-api`} value={base} placeholder={fallback ?? ''} onChange={(e) => setApiBase(e.target.value)} />
      </Field>
      <Field label="Token" htmlFor={`${id}-credential`} className="sm:col-span-2">
        <CredentialSelect id={`${id}-credential`} workspace={workspace.id} kind="forge_token" value={cred} onChange={setCredential} optional={false} />
      </Field>
      <div className="flex flex-wrap items-center gap-2 sm:col-span-2">
        <Button type="submit" disabled={cred === ''}>
          {connection ? 'Preview change' : 'Preview connection'}
        </Button>
        {connection && (
          <>
            <Button type="button" variant="outline" disabled={probing} onClick={test}>
              {probing ? 'Testing…' : 'Test'}
            </Button>
            <Button
              type="button"
              variant="ghost"
              onClick={() =>
                propose('Remove forge connection', { type: 'PreviewForgeRemove', data: { workspace: workspace.id } }, reloadOnConfirm(workspace.id))
              }
            >
              Remove
            </Button>
          </>
        )}
      </div>
      {probe && (
        <p role="status" data-probe={'ok' in probe ? (probe.ok ? 'ok' : 'failed') : 'refused'} className="text-xs sm:col-span-2">
          {'ok' in probe ? (
            probe.ok ? (
              <span className="text-status-ok">
                Connected{probe.can_push ? ', and the token can push.' : ', but the token cannot push.'}
                {probe.message ? ` ${probe.message}` : ''}
              </span>
            ) : (
              <span className="text-status-err">{probe.message ?? 'The forge refused the token.'}</span>
            )
          ) : (
            <span className="text-status-err">{'message' in probe ? probe.message : probe.reason}</span>
          )}
        </p>
      )}
      {!probe && connection?.probe_error && <p className="text-xs text-status-err sm:col-span-2">Last test: {connection.probe_error}</p>}
    </form>
  )
}

function DangerZone({ workspace }: { workspace: WorkspaceInfo }) {
  const propose = useGovernance((s) => s.propose)
  if (workspace.system) return <p className="text-xs text-muted-foreground">The System workspace cannot be archived.</p>
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-2">
      <p className="min-w-0 flex-1 text-xs text-muted-foreground">
        Archiving hides the workspace from the switcher. Its hosts, bindings and runs are kept.
      </p>
      <Button
        variant="destructive"
        size="sm"
        onClick={() => propose('Archive workspace', { type: 'PreviewWorkspaceArchive', data: { workspace: workspace.id } })}
      >
        Archive workspace
      </Button>
    </div>
  )
}

export function WorkspaceTab() {
  const current = useCurrentWorkspace()
  const load = useCallback((workspace: string) => {
    useRepos.getState().load(workspace)
    useForge.getState().load(workspace)
  }, [])

  return (
    <div className="flex min-w-0 flex-col gap-4" data-workspace-settings={current?.slug ?? ''}>
      <CurrentWorkspace load={load} />
      {current && (
        <>
          <Section id="workspace-repository" title={`${current.name}: Repository`}>
            <Repository workspace={current} />
          </Section>
          <Section id="workspace-forge" title={`${current.name}: Forge connection`}>
            <Forge key={current.id} workspace={current} />
          </Section>
          <Section id="workspace-danger" title={`${current.name}: Danger zone`}>
            <DangerZone workspace={current} />
          </Section>
        </>
      )}
    </div>
  )
}
