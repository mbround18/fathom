/**
 * Settings → System, shaped by role (spec 061 FR-024):
 *
 * - Owners: the running build (spec 061), the update card, then spec 060's System workspace.
 * - Admins: the running build and the read-only update line only. The tab never sends `GetSystem`
 *   or any other System workspace command for them.
 *
 * The System workspace (spec 060 US7, owners only), `keel-system`, is where Keel manages its own
 * hosts.
 *
 * Not configured: an empty state with Set up, which creates it allowing the `base` profile.
 * Configured: the profiles it allows (saved with the version they were edited from; a conflict
 * shows a banner with Reload), the bindings with the usual new-binding form, its hosts with
 * Enroll, a link to its runs, and Remove, which needs the workspace's name typed first.
 */
import { HardDrivesIcon, PlayIcon, PlusIcon, ShieldStarIcon, TrashIcon, TreeStructureIcon } from '@phosphor-icons/react'
import { Badge, Button, Input } from '@companion/ui'
import { Link } from '@companion/ui/kit'
import { useEffect, useId, useState } from 'react'

import { NewBinding } from '@/features/bindings/BindingsPage'
import { ErrorLine, PreviewDialog } from '@/features/bindings/parts'
import { useRepos } from '@/features/bindings/store'
import { useSession } from '@/app/session-store'
import { useEnroll } from '@/features/hosts/enroll-store'
import { EmptyState } from '@/ui/EmptyState'
import { LoadState } from '@/ui/LoadState'

import { BuildPanel } from './BuildPanel'
import { ConflictBanner } from './EnrollmentTab'
import { useSettings, type SystemInfo } from './store'
import { UpdateCard } from './UpdateCard'
import { UpdateLine } from './UpdateLine'

/** The System workspace's slug, typed to confirm Remove. */
const SYSTEM_SLUG = 'keel-system'

const H2 = 'font-display text-base font-semibold'
const SECTION = 'flex min-w-0 flex-col gap-2 border border-border p-3'

/** `base, web  db` → `['base', 'web', 'db']`, without repeats. */
function parseProfiles(text: string): string[] {
  return [...new Set(text.split(/[\s,]+/).filter((p) => p !== ''))]
}

function Profiles({ system }: { system: SystemInfo }) {
  const configure = useSettings((s) => s.configureSystem)
  const [text, setText] = useState(system.allowed_profiles.join(', '))
  const id = useId()
  const profiles = parseProfiles(text)
  const changed = profiles.join(',') !== system.allowed_profiles.join(',')
  return (
    <form
      aria-label="System profiles"
      className={SECTION}
      onSubmit={(e) => {
        e.preventDefault()
        if (changed && profiles.length > 0) configure(profiles)
      }}
    >
      <h2 className={H2}>Profiles</h2>
      <p className="text-xs text-muted-foreground">
        The profiles hosts in this workspace may be enrolled with. Separate them with commas or spaces.
      </p>
      <div className="flex min-w-0 flex-wrap items-end gap-2">
        <label htmlFor={id} className="sr-only">
          Allowed profiles
        </label>
        <Input
          id={id}
          value={text}
          spellCheck={false}
          className="min-w-0 flex-1 basis-48 font-mono"
          onChange={(e) => {
            setText(e.target.value)
          }}
        />
        <Button type="submit" size="sm" disabled={!changed || profiles.length === 0}>
          Save
        </Button>
      </div>
      {profiles.length === 0 && <p className="text-xs text-status-err">Allow at least one profile.</p>}
    </form>
  )
}

function Bindings({ workspace }: { workspace: string }) {
  const load = useRepos((s) => s.load)
  const bindings = useRepos((s) => s.bindings)
  const [adding, setAdding] = useState(false)
  useEffect(() => {
    load(workspace)
  }, [load, workspace])
  return (
    <section aria-label="System bindings" className={SECTION}>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <h2 className={H2}>Bindings</h2>
        <span className="flex-1" />
        <Button
          variant="outline"
          size="xs"
          aria-expanded={adding}
          onClick={() => {
            setAdding((a) => !a)
          }}
        >
          <PlusIcon aria-hidden />
          {adding ? 'Close form' : 'New binding'}
        </Button>
      </div>
      <ErrorLine />
      {adding && <NewBinding />}
      {bindings.length === 0 ? (
        <p data-pane-state="empty" className="text-xs text-muted-foreground">
          No repository is bound yet. Bind the repository that holds Keel&apos;s own playbooks.
        </p>
      ) : (
        <ul aria-label="Bindings" className="flex min-w-0 flex-col gap-1 text-xs">
          {bindings.map((b) => (
            <li key={b.id} data-binding={b.id} className="flex min-w-0 flex-wrap items-center gap-2">
              <span className="min-w-0 font-mono break-all">{b.url}</span>
              <span className="text-muted-foreground">{b.branch}</span>
              <Badge variant="outline">{b.status}</Badge>
            </li>
          ))}
        </ul>
      )}
      <Link to="/bindings" className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground">
        <TreeStructureIcon aria-hidden className="size-3.5" />
        Jobs, credentials and sync on Bindings
      </Link>
      <PreviewDialog />
    </section>
  )
}

function Hosts({ system }: { system: SystemInfo }) {
  const open = useEnroll((s) => s.open)
  const workspace = system.workspace
  return (
    <section aria-label="System hosts" className={SECTION}>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <h2 className={H2}>Hosts</h2>
        <span data-system-hosts className="text-xs text-muted-foreground tabular-nums">
          {system.hosts.length}
        </span>
        <span className="flex-1" />
        <Button
          variant="outline"
          size="xs"
          onClick={() => {
            open(workspace)
          }}
        >
          <HardDrivesIcon aria-hidden />
          Enroll host
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        {system.hosts.length === 0
          ? 'No host is enrolled here yet. Enroll the machine Keel runs on to manage it from Keel.'
          : 'Each host enrolled here runs the jobs bound to this workspace.'}
      </p>
      <div className="flex min-w-0 flex-wrap gap-3 text-xs">
        <Link to="/hosts" className="inline-flex items-center gap-1 text-muted-foreground hover:text-foreground">
          <HardDrivesIcon aria-hidden className="size-3.5" />
          See them on Hosts
        </Link>
        <Link to="/runs" className="inline-flex items-center gap-1 text-muted-foreground hover:text-foreground">
          <PlayIcon aria-hidden className="size-3.5" />
          Recent runs
        </Link>
      </div>
    </section>
  )
}

function Remove() {
  const remove = useSettings((s) => s.removeSystem)
  const [typed, setTyped] = useState('')
  const id = useId()
  const ready = typed === SYSTEM_SLUG
  return (
    <form
      aria-label="Remove the System workspace"
      className="flex min-w-0 flex-col gap-2 border border-status-err/50 p-3"
      onSubmit={(e) => {
        e.preventDefault()
        if (ready) remove()
      }}
    >
      <h2 className={`${H2} text-status-err`}>Remove</h2>
      <p className="text-xs text-muted-foreground">
        Revokes its hosts, disables its bindings and archives the workspace. Runs and audit stay. Type{' '}
        <code className="font-mono text-foreground">{SYSTEM_SLUG}</code> to confirm.
      </p>
      <div className="flex min-w-0 flex-wrap items-end gap-2">
        <label htmlFor={id} className="sr-only">
          Type {SYSTEM_SLUG} to confirm
        </label>
        <Input
          id={id}
          value={typed}
          autoComplete="off"
          spellCheck={false}
          placeholder={SYSTEM_SLUG}
          className="min-w-0 flex-1 basis-48 font-mono"
          onChange={(e) => {
            setTyped(e.target.value)
          }}
        />
        <Button type="submit" variant="destructive" size="sm" disabled={!ready}>
          <TrashIcon aria-hidden />
          Remove
        </Button>
      </div>
    </form>
  )
}

function Configured({ system }: { system: SystemInfo }) {
  return (
    <div data-system="configured" className="flex min-w-0 flex-col gap-3">
      {/* A new version starts the profiles editor over from it. */}
      <Profiles key={system.version} system={system} />
      {system.workspace && <Bindings workspace={system.workspace} />}
      <Hosts system={system} />
      <Remove />
    </div>
  )
}

/** Spec 060's System workspace, for owners only. */
function SystemWorkspace() {
  const { value, refusal } = useSettings((s) => s.system)
  const conflict = useSettings((s) => s.systemConflict)
  const load = useSettings((s) => s.loadSystem)
  const configure = useSettings((s) => s.configureSystem)
  useEffect(() => {
    load()
  }, [load])
  return (
    <div id="system-workspace" className="flex min-w-0 scroll-mt-2 flex-col gap-3">
      {conflict && <ConflictBanner what="the System workspace" onReload={load} />}
      <LoadState loading={value === null} failed={refusal}
        what="the System workspace"
        onRetry={load}
        label="Loading the System workspace…">
        {value &&
          (value.configured ? (
            <Configured system={value} />
          ) : (
            <EmptyState
              icon={<ShieldStarIcon aria-hidden />}
              title="No System workspace"
              action={
                <Button
                  size="sm"
                  onClick={() => {
                    configure(['base'])
                  }}
                >
                  Set up
                </Button>
              }
            >
              A workspace only owners can see, for the hosts Keel itself runs on. Set up creates{' '}
              <code className="font-mono">{SYSTEM_SLUG}</code> allowing the <code className="font-mono">base</code> profile;
              you can change that after.
            </EmptyState>
          ))}
      </LoadState>
    </div>
  )
}

export function SystemTab() {
  const role = useSession((s) => s.me?.instance_role)
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <BuildPanel />
      {role === 'owner' ? (
        <>
          <UpdateCard />
          <SystemWorkspace />
        </>
      ) : (
        <UpdateLine />
      )}
    </div>
  )
}
