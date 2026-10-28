/**
 * Hosts (spec 055 US7, T088; spec 060 US1): the workspace's enrolled hosts with their state,
 * revoking one, and the SSH CA line runners pin as `keel_runner_host_ca`. Enrolling a host opens the
 * Enroll dialog from the header, offered when the caller can operate on some workspace.
 */
import { PlusIcon } from '@phosphor-icons/react'
import { Button, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@companion/ui'
import { useEffect, useId, useMemo } from 'react'

import { RefusalText } from '@/features/teams/parts'
import type { HostStatus } from '@/protocol/wire'
import { CopyBlock } from '@/ui/CopyBlock'
import { PageHeader } from '@/ui/PageHeader'

import { EnrollDialog } from './EnrollDialog'
import { useEnroll } from './enroll-store'
import { FIELD, useHosts } from './store'
import { useWorkspaces } from './workspaces'

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

/**
 * Picks one of the workspaces the caller can see (`ListWorkspaces`) and lists its hosts. The System
 * workspace is listed only once it exists, since the server sends it only then.
 */
function WorkspacePicker() {
  const { all, refusal } = useWorkspaces()
  const workspace = useHosts((s) => s.workspace)
  const load = useHosts((s) => s.load)
  const id = useId()
  const first = all?.[0]?.id ?? null
  const known = workspace !== null && all?.some((w) => w.id === workspace) === true

  useEffect(() => {
    const current = known ? workspace : first
    if (current) load(current)
  }, [known, workspace, first, load])

  if (refusal) return <RefusalText refusal={refusal} />
  if (all === null) return null
  if (all.length === 0)
    return (
      <p data-pane-state="empty" className="text-xs text-muted-foreground">
        You have no workspace yet. An owner or admin creates one in Teams.
      </p>
    )
  return (
    <div className="flex min-w-0 items-center gap-2">
      <label htmlFor={id} className="text-[11px] tracking-wider text-muted-foreground uppercase">
        Workspace
      </label>
      <select
        id={id}
        className={`${FIELD} min-w-0`}
        value={known ? workspace : (first ?? '')}
        onChange={(e) => {
          load(e.target.value)
        }}
      >
        {all.map((w) => (
          <option key={w.id} value={w.id}>
            {w.system ? `${w.slug} (System)` : w.slug}
          </option>
        ))}
      </select>
    </div>
  )
}

/** Enroll, in the header: offered when the caller can operate on some workspace. */
function EnrollButton() {
  const { operable } = useWorkspaces()
  const workspace = useHosts((s) => s.workspace)
  const open = useEnroll((s) => s.open)
  if (operable.length === 0) return null
  const start = operable.some((w) => w.id === workspace) ? workspace : null
  return (
    <Button
      size="sm"
      data-enroll-open
      onClick={() => {
        open(start)
      }}
    >
      <PlusIcon aria-hidden />
      Enroll host
    </Button>
  )
}

/** The confirm dialog for a pending revoke. */
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
        <CopyBlock label="keel_runner_host_ca" value={ssh.cert_authority} data="cert-authority" />
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
  const mintedCount = useEnroll((s) => s.mintedCount)
  // Profiles already in use here, offered as chips in the Enroll dialog.
  const suggestions = useMemo(
    () => [...new Set(['base', ...hosts.flatMap((h) => h.profiles)])].sort(),
    [hosts],
  )

  // A token just minted may be for this workspace: re-read so a pending host shows up.
  useEffect(() => {
    if (mintedCount > 0 && workspace) useHosts.getState().load(workspace)
  }, [mintedCount, workspace])

  return (
    <section data-page="hosts" aria-labelledby="hosts-title" className="flex min-w-0 flex-col gap-3 p-4">
      <PageHeader id="hosts-title" title="Hosts" actions={<EnrollButton />}>
        <WorkspacePicker />
      </PageHeader>
      <ErrorLine />
      {workspace && (
        <>
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
                        : 'No hosts yet. Enroll one, then run the snippet on it.'}
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
      <EnrollDialog suggestions={suggestions} />
    </section>
  )
}
