/**
 * Hosts (spec 055 US7, T088; spec 060 US1): the workspace's enrolled hosts with their state,
 * revoking one, and the SSH CA line runners pin as `keel_runner_host_ca`. Enrolling a host opens the
 * Enroll dialog from the header, offered when the caller can operate on some workspace.
 */
import { PlusIcon, XIcon } from '@phosphor-icons/react'
import { Button, Checkbox } from '@companion/ui'
import { useEffect, useId, useMemo } from 'react'

import { useSession } from '@/app/session-store'
import { WorkspaceSelect } from '@/app/WorkspacePicker'
import { RefusalText } from '@/features/teams/parts'
import type { HostInfo, HostStatus } from '@/protocol/wire'
import { ConfirmDialog as Confirm } from '@/ui/ConfirmDialog'
import { CopyBlock } from '@/ui/CopyBlock'
import { DataTable, type Column } from '@/ui/DataTable'
import { EmptyState } from '@/ui/EmptyState'
import { emptyVariant } from '@/ui/empty-variant'
import { ErrorLine as Alert } from '@/ui/ErrorLine'
import { H2, LABEL, LINK, STATUS } from '@/ui/form'
import { LoadState } from '@/ui/LoadState'
import { NoMatch } from '@/ui/NoMatch'
import { Page } from '@/ui/Page'
import { useReveal } from '@/ui/reveal'
import { PageHeader } from '@/ui/PageHeader'

import { EnrollDialog } from './EnrollDialog'
import { useEnroll } from './enroll-store'
import { useHosts } from './store'
import { canOperate, useWorkspaces } from './workspaces'

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
  return <span className={`${STATUS} ${tone(state)}`}>{state}</span>
}

/** The host reported a run Keel did not make (058 A2). */
function ChangedBadge() {
  return (
    <span
      data-changed-outside
      title="The host reported an ansible-playbook run that Keel did not make."
      className="border border-status-warn/50 px-1.5 text-xs font-medium whitespace-nowrap text-status-warn"
    >
      Changed outside Keel
    </span>
  )
}

/** Narrows the list to hosts changed outside Keel. */
function ChangedFilter() {
  const on = useHosts((s) => s.changedOnly)
  const set = useHosts((s) => s.setChangedOnly)
  const id = useId()
  return (
    <div className="flex min-w-0 items-center gap-2 text-xs">
      <Checkbox
        id={id}
        checked={on}
        onCheckedChange={(c) => {
          set(c === true)
        }}
      />
      <label htmlFor={id}>Only hosts changed outside Keel</label>
    </div>
  )
}

/** One host's detail: what it reported was run on it outside Keel, as its claims. */
function HostDetailPanel() {
  const open = useHosts((s) => s.open)
  const detail = useHosts((s) => s.detail)
  const show = useHosts((s) => s.show)
  const reveal = useReveal<HTMLElement>(open)
  if (open === null) return null
  const name = detail?.host.inventory_name ?? '…'
  return (
    <section ref={reveal} aria-label={`Host ${name}`} className="flex min-w-0 scroll-mt-3 flex-col border border-border text-xs">
      <div className="flex min-h-10 min-w-0 flex-wrap items-center gap-2 border-b border-border bg-muted/20 px-3 py-1.5">
        <h2 className="min-w-0 font-mono text-sm break-all">{name}</h2>
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
          <XIcon aria-hidden />
          Close
        </Button>
      </div>
      {detail && (
        <div className="flex min-w-0 flex-col gap-2 p-3">
          <h3 className={LABEL}>Runs outside Keel</h3>
          <p className="max-w-prose text-muted-foreground">
            What {name} reports was run on it without Keel. These are the host&apos;s claims: Keel did not
            make these runs and cannot verify them.
          </p>
          {detail.outside_runs.length === 0 ? (
            <p data-pane-state="empty" className="border border-dashed border-border px-3 py-3 text-muted-foreground">
              The host has reported none.
            </p>
          ) : (
            <ul aria-label="Runs outside Keel" className="flex min-w-0 flex-col border border-border">
              {detail.outside_runs.map((r) => (
                <li
                  key={`${r.at}-${r.playbook}-${r.commit ?? ''}`}
                  data-outside-check={r.check}
                  className="flex min-w-0 flex-wrap gap-x-3 border-t border-border px-2 py-1 font-mono first:border-t-0"
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
        </div>
      )}
    </section>
  )
}

function ErrorLine() {
  const error = useHosts((s) => s.error)
  const clear = useHosts((s) => s.clearError)
  return <Alert message={error} onDismiss={clear} />
}

/**
 * Picks one of the workspaces the caller can see (`ListWorkspaces`) and lists its hosts. The System
 * workspace is listed only once it exists, since the server sends it only then.
 */
function WorkspacePicker() {
  const { all, refusal } = useWorkspaces()
  const workspace = useHosts((s) => s.workspace)
  const load = useHosts((s) => s.load)
  const first = all?.[0]?.id ?? null
  const known = workspace !== null && all?.some((w) => w.id === workspace) === true

  useEffect(() => {
    const current = known ? workspace : first
    if (current) load(current)
  }, [known, workspace, first, load])

  if (refusal) return <RefusalText refusal={refusal} />
  if (all === null) return null
  return (
    <WorkspaceSelect
      options={all.map((w) => ({ id: w.id, label: w.system ? `${w.slug} (System)` : w.slug }))}
      value={known ? workspace : (first ?? '')}
      onChange={load}
    />
  )
}

/**
 * Enroll, in the header: offered when the caller can operate on some workspace. `quiet` while the
 * empty list's own Enroll button is the page's one primary action.
 */
function EnrollButton({ quiet }: { quiet: boolean }) {
  const { operable } = useWorkspaces()
  const workspace = useHosts((s) => s.workspace)
  const open = useEnroll((s) => s.open)
  if (operable.length === 0) return null
  const start = operable.some((w) => w.id === workspace) ? workspace : null
  return (
    <Button
      size="sm"
      variant={quiet ? 'outline' : 'default'}
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

/** No hosts in the workspace yet (spec 063 FR-011, FR-012). */
function NoHosts({ workspace, canAct }: { workspace: string; canAct: boolean }) {
  const open = useEnroll((s) => s.open)
  return (
    <EmptyState
      page="hosts"
      title={canAct ? 'Enroll a host' : 'No hosts yet'}
      reason="Keel only runs on hosts that have enrolled."
      steps={[
        'Enroll a host to get its one-time command.',
        'Run the command on the host.',
        'Run the base profile on it.',
      ]}
      action={
        canAct ? (
          <Button
            size="sm"
            onClick={() => {
              open(workspace)
            }}
          >
            <PlusIcon aria-hidden />
            Enroll a host
          </Button>
        ) : undefined
      }
      whoCan="An operator or workspace admin"
      verb="enroll hosts"
      guide={{ guide: 'hosts', anchor: 'enroll-a-host' }}
    />
  )
}

/** The confirm dialog for a pending revoke. */
function ConfirmDialog() {
  const preview = useHosts((s) => s.preview)
  const confirm = useHosts((s) => s.confirm)
  const cancel = useHosts((s) => s.cancel)
  return <Confirm preview={preview} onCancel={cancel} onConfirm={() => void confirm()} />
}

/** The host CA line runners pin. */
function SshCaLine() {
  const ssh = useHosts((s) => s.ssh)
  if (!ssh) return null
  return (
    <details className="border border-border text-xs">
      <summary
        className={`flex min-h-10 cursor-pointer items-center px-3 py-1.5 select-none hover:bg-muted/30 focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none focus-visible:ring-inset ${H2}`}
      >
        SSH certificate authority
      </summary>
      <div className="flex min-w-0 flex-col gap-2 border-t border-border p-3">
        <p className="max-w-prose text-muted-foreground">
          Runners pin this line as <code className="font-mono">keel_runner_host_ca</code> to trust the hosts Keel
          enrolled.
        </p>
        <CopyBlock label="keel_runner_host_ca" value={ssh.cert_authority} data="cert-authority" />
        <ul aria-label="SSH CAs" className="flex min-w-0 flex-col gap-1">
          {ssh.cas.map((ca) => (
            <li key={ca.id} className="font-mono break-all text-muted-foreground">
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
  const opened = useHosts((s) => s.open)
  const workspace = useHosts((s) => s.workspace)
  const revoke = useHosts((s) => s.revoke)
  const show = useHosts((s) => s.show)
  const changedOnly = useHosts((s) => s.changedOnly)
  const setChangedOnly = useHosts((s) => s.setChangedOnly)
  const loaded = useHosts((s) => s.loaded)
  const me = useSession((s) => s.me)
  const mintedCount = useEnroll((s) => s.mintedCount)
  // Profiles already in use here, offered as chips in the Enroll dialog.
  const suggestions = useMemo(
    () => [...new Set(['base', ...hosts.flatMap((h) => h.profiles)])].sort(),
    [hosts],
  )

  const columns = useMemo<Column<HostInfo>[]>(
    () => [
      {
        key: 'name',
        header: 'Name',
        className: 'w-[28%]',
        cell: (h) => (
          <span className="flex min-w-0 flex-wrap items-center gap-1">
            <button
              type="button"
              className={`${LINK} font-mono break-all`}
              aria-label={`Show ${h.inventory_name}`}
              onClick={() => {
                show(h.id)
              }}
            >
              {h.inventory_name}
            </button>
            {h.changed_outside && <ChangedBadge />}
          </span>
        ),
      },
      { key: 'state', header: 'State', cell: (h) => <StateBadge state={h.state} /> },
      { key: 'profiles', header: 'Profiles', cell: (h) => <span className="font-mono">{h.profiles.join(', ')}</span> },
      { key: 'runner', header: 'Runner', cell: (h) => h.runner_version ?? '—' },
      { key: 'ansible', header: 'Ansible', cell: (h) => h.ansible_version ?? '—' },
      { key: 'seen', header: 'Last seen', cell: (h) => <span className="tabular-nums">{h.last_seen_at ?? '—'}</span> },
    ],
    [show],
  )

  // A token just minted may be for this workspace: re-read so a pending host shows up.
  useEffect(() => {
    if (mintedCount > 0 && workspace) useHosts.getState().load(workspace)
  }, [mintedCount, workspace])

  const variant = emptyVariant({ filtered: changedOnly, canAct: workspace !== null && canOperate(me, workspace) })
  // The guided state replaces the table only once the list has answered empty and unfiltered.
  const guided = loaded && hosts.length === 0 && variant !== 'no-match'

  return (
    <Page name="hosts" labelledBy="hosts-title">
      <PageHeader
        id="hosts-title"
        title="Hosts"
        description="The machines enrolled in a workspace, their state, and what ran on them outside Keel."
        actions={<EnrollButton quiet={guided} />}
      >
        <WorkspacePicker />
      </PageHeader>
      <ErrorLine />
      {workspace && guided && <NoHosts workspace={workspace} canAct={variant === 'guided'} />}
      {workspace && !guided && (
        <>
          <DataTable
            label="Hosts"
            filter={<ChangedFilter />}
            columns={columns}
            rows={hosts}
            rowKey={(h) => h.id}
            selected={(h) => h.id === opened}
            rowData={(h) => ({ 'data-host-state': h.state, 'data-host-changed-outside': String(h.changed_outside) })}
            actions={(h) =>
              h.state !== 'revoked' && (
                <Button
                  variant="ghost"
                  size="xs"
                  className="text-status-err"
                  aria-label={`Revoke ${h.inventory_name}`}
                  onClick={() => {
                    revoke(h.id)
                  }}
                >
                  Revoke
                </Button>
              )
            }
            empty={
              loaded ? (
                <NoMatch
                  what="hosts"
                  onClear={() => {
                    setChangedOnly(false)
                  }}
                />
              ) : (
                <LoadState loading label="Loading hosts…" />
              )
            }
          />
          <HostDetailPanel />
        </>
      )}
      {workspace && <SshCaLine />}
      <ConfirmDialog />
      <EnrollDialog suggestions={suggestions} />
    </Page>
  )
}
