/**
 * Bindings (spec 055 US1, T052): repositories bound to a project, the jobs their
 * `.keel/jobs.yaml` defines at the last synced commit, and the workspace's credentials.
 * Every change is previewed and confirmed (./parts.tsx); credential values are sent once and
 * never shown back, only their fingerprints.
 */
import { Button, Input } from '@companion/ui'
import { useId, useState, type SyntheticEvent } from 'react'

import type { BindingInfo, CredentialKind, Transport } from '@/protocol/wire'

import { ErrorLine, Field, PreviewDialog, WorkspacePicker } from './parts'
import { FIELD, short, useRepos } from './store'

const KINDS: { kind: CredentialKind; label: string }[] = [
  { kind: 'git_https_token', label: 'Git HTTPS token' },
  { kind: 'git_ssh_key', label: 'Git SSH key' },
  { kind: 'vault_password', label: 'Vault password' },
  { kind: 'env', label: 'Environment' },
  { kind: 'secret_vars', label: 'Secret vars' },
]

function NewBinding() {
  const propose = useRepos((s) => s.propose)
  const credentials = useRepos((s) => s.credentials)
  const [project, setProject] = useState('')
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
        project: project.trim(),
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
    <form aria-label="New binding" onSubmit={submit} className="grid gap-2 border border-border p-3 sm:grid-cols-2">
      <Field label="Project id" id={`${id}-project`}>
        <Input id={`${id}-project`} value={project} onChange={(e) => setProject(e.target.value)} required />
      </Field>
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
      <Field label="Poll every (seconds, blank for webhook only)" id={`${id}-poll`}>
        <Input id={`${id}-poll`} inputMode="numeric" value={poll} onChange={(e) => setPoll(e.target.value)} />
      </Field>
      <div className="flex items-end">
        <Button type="submit">Preview binding</Button>
      </div>
    </form>
  )
}

function Detail({ binding }: { binding: BindingInfo }) {
  const detail = useRepos((s) => s.detail)
  const credentials = useRepos((s) => s.credentials)
  const propose = useRepos((s) => s.propose)
  const sync = useRepos((s) => s.sync)
  const open = useRepos((s) => s.open)
  const [grant, setGrant] = useState('')
  const id = useId()
  const jobs = detail?.binding.id === binding.id ? detail.jobs : []
  const granted = credentials.filter((c) => c.grants.some((g) => g.binding === binding.id))

  return (
    <section aria-label={`Binding ${binding.url}`} className="flex flex-col gap-3 border border-border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-mono text-xs break-all">
          {binding.url} @ {binding.branch}
        </h2>
        <span data-binding-status className="text-[11px] uppercase">
          {binding.status}
        </span>
        <div className="ml-auto flex flex-wrap gap-1">
          <Button size="xs" variant="outline" onClick={() => sync(binding.id)}>
            Sync now
          </Button>
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
          <Button size="xs" variant="ghost" onClick={() => open(null)}>
            Close
          </Button>
        </div>
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt className="text-muted-foreground">Last commit</dt>
        <dd className="font-mono">{short(binding.last_commit)}</dd>
        <dt className="text-muted-foreground">Synced</dt>
        <dd>{binding.last_synced_at ?? 'never'}</dd>
        <dt className="text-muted-foreground">Polling</dt>
        <dd>{binding.poll_interval === null ? 'webhook only' : `every ${String(binding.poll_interval)}s`}</dd>
        {binding.last_sync_error && (
          <>
            <dt className="text-muted-foreground">Error</dt>
            <dd role="status" data-sync-error className="text-status-err">
              {binding.last_sync_error}
            </dd>
          </>
        )}
      </dl>
      {binding.manifest_problems.length > 0 && (
        <ul aria-label="Manifest problems" className="flex flex-col gap-1 text-xs text-status-err">
          {binding.manifest_problems.map((p, i) => (
            <li key={i}>{JSON.stringify(p)}</li>
          ))}
        </ul>
      )}
      <table aria-label="Jobs" className="w-full text-left text-xs">
        <thead className="text-muted-foreground">
          <tr>
            <th className="py-1">Job</th>
            <th>Playbook</th>
            <th>Profiles</th>
            <th>Inventory</th>
            <th>State</th>
          </tr>
        </thead>
        <tbody>
          {jobs.length === 0 && (
            <tr>
              <td colSpan={5} data-pane-state="empty" className="py-1 text-muted-foreground">
                No jobs at this commit.
              </td>
            </tr>
          )}
          {jobs.map((j) => (
            <tr key={j.name} className="border-t border-border">
              <td className="py-1 font-mono">{j.name}</td>
              <td className="font-mono">{j.playbook}</td>
              <td>{j.profiles.join(', ')}</td>
              <td>{j.inventory}</td>
              <td>{j.blocked ? <span className="text-status-err">blocked: {j.blocked}</span> : 'ready'}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="flex flex-wrap items-end gap-2">
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
    </section>
  )
}

function Credentials() {
  const workspace = useRepos((s) => s.workspace)
  const credentials = useRepos((s) => s.credentials)
  const propose = useRepos((s) => s.propose)
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
    <section aria-labelledby={`${id}-title`} className="flex flex-col gap-2">
      <h2 id={`${id}-title`} className="font-display text-xs font-semibold tracking-wider uppercase">
        Credentials
      </h2>
      <table aria-label="Credentials" className="w-full text-left text-xs">
        <thead className="text-muted-foreground">
          <tr>
            <th className="py-1">Name</th>
            <th>Kind</th>
            <th>Fingerprint</th>
            <th>Granted to</th>
          </tr>
        </thead>
        <tbody>
          {credentials.map((c) => (
            <tr key={c.id} className="border-t border-border">
              <td className="py-1 font-mono">{c.name}</td>
              <td>{c.kind}</td>
              <td className="font-mono">{c.fingerprint}</td>
              <td>{c.grants.length}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <form aria-label="New credential" onSubmit={submit} className="flex flex-wrap items-end gap-2">
        <Field label="Name" id={`${id}-name`}>
          <Input id={`${id}-name`} value={name} onChange={(e) => setName(e.target.value)} required />
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
    </section>
  )
}

export default function BindingsPage() {
  const bindings = useRepos((s) => s.bindings)
  const detail = useRepos((s) => s.detail)
  const workspace = useRepos((s) => s.workspace)
  const open = useRepos((s) => s.open)
  const [adding, setAdding] = useState(false)
  const current = detail ? (bindings.find((b) => b.id === detail.binding.id) ?? detail.binding) : null

  return (
    <section data-page="bindings" aria-labelledby="bindings-title" className="flex flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 id="bindings-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Bindings
        </h1>
        <WorkspacePicker />
        {workspace && (
          <Button size="sm" variant="outline" className="ml-auto" onClick={() => setAdding((a) => !a)}>
            {adding ? 'Close form' : 'New binding'}
          </Button>
        )}
      </div>
      <ErrorLine />
      {adding && <NewBinding />}
      {workspace && (
        <table aria-label="Bindings" className="w-full text-left text-xs">
          <thead className="text-muted-foreground">
            <tr>
              <th className="py-1">Repository</th>
              <th>Branch</th>
              <th>Status</th>
              <th>Commit</th>
              <th>Error</th>
            </tr>
          </thead>
          <tbody>
            {bindings.length === 0 && (
              <tr>
                <td colSpan={5} data-pane-state="empty" className="py-1 text-muted-foreground">
                  No repository is bound yet.
                </td>
              </tr>
            )}
            {bindings.map((b) => (
              <tr key={b.id} className="border-t border-border">
                <td className="py-1">
                  <button type="button" className="font-mono break-all hover:underline" onClick={() => open(b.id)}>
                    {b.url}
                  </button>
                </td>
                <td>{b.branch}</td>
                <td>{b.status}</td>
                <td className="font-mono">{short(b.last_commit)}</td>
                <td className="text-status-err">{b.last_sync_error ?? ''}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {current && <Detail binding={current} />}
      {workspace && <Credentials />}
      <PreviewDialog />
    </section>
  )
}
