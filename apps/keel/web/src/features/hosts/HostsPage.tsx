/**
 * Hosts (spec 055 US7, T088): the workspace's enrolled hosts with their state, enrolling a new one
 * (a one-time token, shown once, for the base profile's `keel_enroll_token`), revoking one, and the
 * SSH CA line runners pin as `keel_runner_host_ca`.
 */
import { Button, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@companion/ui'
import { useEffect, useId, useState } from 'react'

import { useSession } from '@/app/session-store'
import type { HostStatus } from '@/protocol/wire'

import { FIELD, parseProfiles, tokenArg, useHosts } from './store'

function tone(state: HostStatus): string {
  switch (state) {
    case 'online':
      return 'text-status-ok'
    case 'revoked':
      return 'text-status-err'
    default:
      return 'text-muted-foreground'
  }
}

function StateBadge({ state }: { state: HostStatus }) {
  return <span className={`text-[11px] uppercase ${tone(state)}`}>{state}</span>
}

/** The host reported a run Keel did not make (058 A2). */
function ChangedBadge() {
  return (
    <span
      data-changed-outside
      title="The host reported an ansible-playbook run that Keel did not make."
      className="border border-status-warn/50 px-1 text-[11px] whitespace-nowrap text-status-warn uppercase"
    >
      Changed outside Keel
    </span>
  )
}

/** Narrows the list to hosts changed outside Keel. */
function ChangedFilter() {
  const on = useHosts((s) => s.changedOnly)
  const set = useHosts((s) => s.setChangedOnly)
  return (
    <label className="flex items-center gap-2 text-xs">
      <input
        type="checkbox"
        checked={on}
        onChange={(e) => {
          set(e.target.checked)
        }}
      />
      Only hosts changed outside Keel
    </label>
  )
}

/** One host's detail: what it reported was run on it outside Keel, as its claims. */
function HostDetailPanel() {
  const open = useHosts((s) => s.open)
  const detail = useHosts((s) => s.detail)
  const show = useHosts((s) => s.show)
  if (open === null) return null
  const name = detail?.host.inventory_name ?? '…'
  return (
    <section aria-label={`Host ${name}`} className="flex flex-col gap-2 border border-border p-3 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-mono text-sm">{name}</h2>
        {detail && <StateBadge state={detail.host.state} />}
        {detail?.host.changed_outside && <ChangedBadge />}
        <Button
          variant="ghost"
          size="xs"
          className="ml-auto"
          onClick={() => {
            show(null)
          }}
        >
          Close
        </Button>
      </div>
      {detail && (
        <>
          <h3 className="text-muted-foreground uppercase">Runs outside Keel</h3>
          <p className="text-muted-foreground">
            What {name} reports was run on it without Keel. These are the host&apos;s claims: Keel did not
            make these runs and cannot verify them.
          </p>
          {detail.outside_runs.length === 0 ? (
            <p data-pane-state="empty" className="text-muted-foreground">
              The host has reported none.
            </p>
          ) : (
            <ul aria-label="Runs outside Keel" className="flex flex-col gap-1">
              {detail.outside_runs.map((r) => (
                <li
                  key={`${r.at}-${r.playbook}-${r.commit ?? ''}`}
                  data-outside-check={r.check}
                  className="flex flex-wrap gap-x-3 font-mono"
                >
                  <span className="whitespace-nowrap">{r.at}</span>
                  <span>host says: {r.user}</span>
                  <span className="break-all">{r.playbook}</span>
                  <span>{r.commit ? r.commit.slice(0, 12) : 'no commit'}</span>
                  {r.dirty && <span className="text-status-warn">uncommitted changes</span>}
                  {r.check && <span className="text-muted-foreground">--check</span>}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  )
}

function ErrorLine() {
  const error = useHosts((s) => s.error)
  const clear = useHosts((s) => s.clearError)
  if (!error) return null
  return (
    <p role="alert" className="flex items-center justify-between gap-2 border border-status-err/40 px-2 py-1 text-xs text-status-err">
      <span>{error}</span>
      <Button variant="ghost" size="xs" onClick={clear}>
        Dismiss
      </Button>
    </p>
  )
}

/** Picks one of the signed-in user's workspaces and lists its hosts. */
function WorkspacePicker() {
  const grants = useSession((s) => s.me?.grants ?? [])
  const workspace = useHosts((s) => s.workspace)
  const load = useHosts((s) => s.load)
  const id = useId()
  const first = grants[0]?.workspace ?? null
  const unique = [...new Map(grants.map((g) => [g.workspace, g.workspace_slug])).entries()]

  useEffect(() => {
    const current = workspace ?? first
    if (current) load(current)
  }, [workspace, first, load])

  if (unique.length === 0)
    return (
      <p data-pane-state="empty" className="text-xs text-muted-foreground">
        You have no workspace yet. An owner or admin grants one from Teams.
      </p>
    )
  return (
    <div className="flex items-center gap-2">
      <label htmlFor={id} className="text-[11px] tracking-wider text-muted-foreground uppercase">
        Workspace
      </label>
      <select
        id={id}
        className={FIELD}
        value={workspace ?? first ?? ''}
        onChange={(e) => {
          load(e.target.value)
        }}
      >
        {unique.map(([ws, slug]) => (
          <option key={ws} value={ws}>
            {slug}
          </option>
        ))}
      </select>
    </div>
  )
}

/** A value with a copy button. */
function Copyable({ label, value, data }: { label: string; value: string; data?: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-2">
        <span className="text-muted-foreground">{label}</span>
        <Button
          variant="outline"
          size="xs"
          aria-label={`Copy ${label.toLowerCase()}`}
          onClick={() => {
            navigator.clipboard.writeText(value).then(
              () => {
                setCopied(true)
              },
              () => {
                setCopied(false)
              },
            )
          }}
        >
          {copied ? 'Copied' : 'Copy'}
        </Button>
      </div>
      <code data-copy={data} className="block border border-border bg-background p-2 font-mono break-all">
        {value}
      </code>
    </div>
  )
}

/** The token just minted: shown once, until dismissed. */
function Token() {
  const enrollment = useHosts((s) => s.enrollment)
  const dismiss = useHosts((s) => s.dismissEnrollment)
  if (!enrollment) return null
  return (
    <section aria-label="Enrolment token" className="flex flex-col gap-2 border border-accent-neon p-3 text-xs">
      <p className="font-semibold">Copy the token now. Keel will not show it again.</p>
      <p className="text-muted-foreground">
        Run the base profile against <span className="font-mono">{enrollment.inventory_name}</span> with this extra var
        before {enrollment.expires_at}. It works once.
      </p>
      <Copyable label="Extra var" value={tokenArg(enrollment.token)} data="enroll-arg" />
      <p className="text-muted-foreground">
        Profiles: <span className="font-mono">{enrollment.profiles.join(', ')}</span>. The host&apos;s
        keel_runner_allowed_profiles must include them.
      </p>
      <p className="text-muted-foreground">
        The runner reads its server address only from <span className="font-mono">KEEL_RUNNER_SERVER</span> in
        runner.env (this server&apos;s runner listener, host:port); policy.toml has no server key.
      </p>
      <div>
        <Button variant="outline" size="xs" onClick={dismiss}>
          Done
        </Button>
      </div>
    </section>
  )
}

/** The enrol form: an inventory name and the profiles to assign. */
function EnrollForm() {
  const enroll = useHosts((s) => s.enroll)
  const [name, setName] = useState('')
  const [profiles, setProfiles] = useState('base')
  const nameId = useId()
  const profilesId = useId()
  const list = parseProfiles(profiles)
  return (
    <form
      aria-label="Enrol a host"
      className="flex flex-wrap items-end gap-2"
      onSubmit={(e) => {
        e.preventDefault()
        if (name.trim() && list.length > 0) enroll(name.trim(), list)
      }}
    >
      <div className="flex flex-col gap-1">
        <label htmlFor={nameId} className="text-[11px] tracking-wider text-muted-foreground uppercase">
          Inventory name
        </label>
        <input
          id={nameId}
          className={FIELD}
          value={name}
          autoComplete="off"
          placeholder="web-1"
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
      </div>
      <div className="flex flex-col gap-1">
        <label htmlFor={profilesId} className="text-[11px] tracking-wider text-muted-foreground uppercase">
          Profiles
        </label>
        <input
          id={profilesId}
          className={FIELD}
          value={profiles}
          autoComplete="off"
          onChange={(e) => {
            setProfiles(e.target.value)
          }}
        />
      </div>
      <Button type="submit" size="sm" disabled={!name.trim() || list.length === 0}>
        Enrol
      </Button>
    </form>
  )
}

/** The confirm dialog for a pending enrol or revoke. */
function ConfirmDialog() {
  const preview = useHosts((s) => s.preview)
  const confirm = useHosts((s) => s.confirm)
  const cancel = useHosts((s) => s.cancel)
  return (
    <Dialog
      open={preview !== null}
      onOpenChange={(open) => {
        if (!open) cancel()
      }}
    >
      {preview && (
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Confirm</DialogTitle>
            <DialogDescription data-preview-summary>{preview.summary}</DialogDescription>
          </DialogHeader>
          {preview.affected.length > 0 && (
            <ul aria-label="Affected" className="flex flex-col gap-1 text-xs">
              {preview.affected.map((a) => (
                <li key={a} className="font-mono break-all">
                  {a}
                </li>
              ))}
            </ul>
          )}
          {preview.step_up && <p className="text-xs text-muted-foreground">This change asks for your passkey.</p>}
          <DialogFooter>
            <Button variant="outline" onClick={cancel}>
              Cancel
            </Button>
            <Button onClick={() => void confirm()}>Confirm</Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  )
}

/** The host CA line runners pin. */
function SshCaLine() {
  const ssh = useHosts((s) => s.ssh)
  if (!ssh) return null
  return (
    <details className="text-xs">
      <summary className="cursor-pointer text-muted-foreground">SSH certificate authority</summary>
      <div className="mt-2 flex flex-col gap-2">
        <Copyable label="keel_runner_host_ca" value={ssh.cert_authority} data="cert-authority" />
        <ul aria-label="SSH CAs" className="flex flex-col gap-1">
          {ssh.cas.map((ca) => (
            <li key={ca.id} className="font-mono break-all">
              {ca.kind} ({ca.state}): {ca.fingerprint}
            </li>
          ))}
        </ul>
      </div>
    </details>
  )
}

export default function HostsPage() {
  const hosts = useHosts((s) => s.hosts)
  const workspace = useHosts((s) => s.workspace)
  const revoke = useHosts((s) => s.revoke)
  const show = useHosts((s) => s.show)
  const changedOnly = useHosts((s) => s.changedOnly)

  return (
    <section data-page="hosts" aria-labelledby="hosts-title" className="flex flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 id="hosts-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Hosts
        </h1>
        <WorkspacePicker />
      </div>
      <ErrorLine />
      <Token />
      {workspace && (
        <>
          <EnrollForm />
          <ChangedFilter />
          <div className="overflow-x-auto">
            <table aria-label="Hosts" className="w-full text-left text-xs">
              <thead className="text-muted-foreground">
                <tr>
                  <th className="py-1">Name</th>
                  <th>State</th>
                  <th>Profiles</th>
                  <th>Runner</th>
                  <th>Ansible</th>
                  <th>Last seen</th>
                  <th>
                    <span className="sr-only">Actions</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {hosts.length === 0 && (
                  <tr>
                    <td colSpan={7} data-pane-state="empty" className="py-1 text-muted-foreground">
                      {changedOnly
                        ? 'No host reports a run outside Keel.'
                        : 'No hosts yet. Enrol one, then run the base profile on it with the token.'}
                    </td>
                  </tr>
                )}
                {hosts.map((h) => (
                  <tr
                    key={h.id}
                    data-host-state={h.state}
                    data-host-changed-outside={h.changed_outside}
                    className="border-t border-border"
                  >
                    <td className="py-1">
                      <span className="flex flex-wrap items-center gap-2">
                        <Button
                          variant="ghost"
                          size="xs"
                          className="font-mono"
                          aria-label={`Show ${h.inventory_name}`}
                          onClick={() => {
                            show(h.id)
                          }}
                        >
                          {h.inventory_name}
                        </Button>
                        {h.changed_outside && <ChangedBadge />}
                      </span>
                    </td>
                    <td>
                      <StateBadge state={h.state} />
                    </td>
                    <td className="font-mono">{h.profiles.join(', ')}</td>
                    <td>{h.runner_version ?? '—'}</td>
                    <td>{h.ansible_version ?? '—'}</td>
                    <td className="whitespace-nowrap">{h.last_seen_at ?? '—'}</td>
                    <td className="text-right">
                      {h.state !== 'revoked' && (
                        <Button
                          variant="ghost"
                          size="xs"
                          aria-label={`Revoke ${h.inventory_name}`}
                          onClick={() => {
                            revoke(h.id)
                          }}
                        >
                          Revoke
                        </Button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <HostDetailPanel />
          <SshCaLine />
        </>
      )}
      <ConfirmDialog />
    </section>
  )
}
