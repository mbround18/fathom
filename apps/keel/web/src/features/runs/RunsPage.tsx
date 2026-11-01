/**
 * Runs (spec 055 US2, T072): the workspace's runs, newest first. Each opens its own page
 * (./RunPage.tsx) with its live output and per-host results. "Waiting for approval" (spec 064 US4)
 * narrows the list to held runs, oldest first, and is kept in `?state=pending_approval`.
 */
import { Button } from '@companion/ui'
import { useCallback, useMemo } from 'react'

import { Link } from '@/app/Link'
import { navigate, useSearchParam } from '@/app/router'
import { useSession } from '@/app/session-store'
import { GrantWorkspacePicker } from '@/app/WorkspacePicker'
import { canOperate } from '@/features/hosts/workspaces'
import type { RunInfo } from '@/protocol/wire'
import { DataTable } from '@/ui/DataTable'
import { EmptyState } from '@/ui/EmptyState'
import { emptyVariant } from '@/ui/empty-variant'
import { LINK } from '@/ui/form'
import { LoadFailed } from '@/ui/LoadFailed'
import { LoadState } from '@/ui/LoadState'
import { NoMatch } from '@/ui/NoMatch'
import { Page } from '@/ui/Page'
import { PageHeader } from '@/ui/PageHeader'

import { waitingText } from './approval'
import { ErrorLine, StateBadge } from './parts'
import { short, useRuns } from './store'

/** No runs in the workspace yet (spec 063 FR-011, FR-012). */
function NoRuns({ canAct }: { canAct: boolean }) {
  return (
    <EmptyState
      page="runs"
      title="No runs yet"
      reason="A run applies a bound branch to its hosts."
      steps={['Bind a repository.', 'Push to the bound branch.', 'Approve the run if Keel asks.']}
      action={
        canAct ? (
          <Button asChild size="sm">
            <Link to="/bindings">Go to bindings</Link>
          </Button>
        ) : undefined
      }
      whoCan="An operator or workspace admin"
      verb="bind repositories"
      guide={{ guide: 'runs' }}
    />
  )
}

type Filter = 'all' | 'pending_approval'

const FILTERS: readonly { value: Filter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'pending_approval', label: 'Waiting for approval' },
]

const SEGMENT =
  'h-7 px-2.5 text-xs text-muted-foreground hover:text-foreground focus-visible:ring-1 focus-visible:ring-accent-neon ' +
  'focus-visible:outline-none aria-pressed:bg-muted aria-pressed:text-foreground'

/** "All" or "Waiting for approval" (spec 064 US4), kept in `?state=` so the view can be linked. */
function RunsFilter({ value }: { value: Filter }) {
  return (
    <div role="group" aria-label="Show runs" data-runs-filter={value} className="inline-flex shrink-0 self-start border border-border">
      {FILTERS.map((f) => (
        <button
          key={f.value}
          type="button"
          aria-pressed={value === f.value}
          className={`${SEGMENT} border-l border-border first:border-l-0`}
          onClick={() => {
            navigate(f.value === 'all' ? '/runs' : `/runs?state=${f.value}`, { replace: true })
          }}
        >
          {f.label}
        </button>
      ))}
    </div>
  )
}

/** Held runs, oldest first: the one waiting longest is the one to answer first. */
function oldestFirst(runs: readonly RunInfo[]): RunInfo[] {
  return runs.filter((r) => r.state === 'pending_approval').sort((a, b) => a.created_at.localeCompare(b.created_at))
}

export default function RunsPage() {
  const runs = useRuns((s) => s.runs)
  const loaded = useRuns((s) => s.loaded)
  const failed = useRuns((s) => s.failed)
  const workspace = useRuns((s) => s.workspace)
  const loadRuns = useRuns((s) => s.load)
  const me = useSession((s) => s.me)
  const filter: Filter = useSearchParam('state') === 'pending_approval' ? 'pending_approval' : 'all'
  const held = filter === 'pending_approval'
  // The picker reloads whenever `load` changes, so a new filter lists again.
  const load = useCallback((ws: string) => loadRuns(ws, held ? 'pending_approval' : null), [loadRuns, held])
  // A run approved while the filter is on stays in the store until the next list: hide it here.
  const shown = useMemo(() => (held ? oldestFirst(runs) : runs), [held, runs])
  const variant = emptyVariant({ filtered: held, canAct: workspace !== null && canOperate(me, workspace) })

  return (
    <Page name="runs" labelledBy="runs-title">
      <PageHeader
        id="runs-title"
        title="Runs"
        description={
          held
            ? 'Runs waiting for approval, the longest-waiting first.'
            : 'Every job run in the workspace, newest first, with its live output.'
        }
      >
        <GrantWorkspacePicker workspace={workspace} load={load} />
      </PageHeader>
      <RunsFilter value={filter} />
      <ErrorLine />
      {workspace && failed && (
        <LoadFailed
          what="runs"
          refusal={failed}
          onRetry={() => {
            load(workspace)
          }}
        />
      )}
      {workspace && !failed && loaded && shown.length === 0 && variant === 'no-match' && (
        <NoMatch
          what="runs"
          message="No runs waiting for approval."
          onClear={() => {
            navigate('/runs', { replace: true })
          }}
        />
      )}
      {workspace && !failed && loaded && shown.length === 0 && variant !== 'no-match' && (
        <NoRuns canAct={variant === 'guided'} />
      )}
      {workspace && !failed && !(loaded && shown.length === 0) && (
        <DataTable<RunInfo>
          label="Runs"
          rows={shown}
          rowKey={(r) => r.id}
          rowData={(r) => ({ 'data-run-state': r.state })}
          columns={[
            {
              key: 'job',
              header: 'Job',
              className: 'w-[22%]',
              cell: (r) => (
                <Link to={`/runs/${r.id}`} className={`${LINK} font-mono break-all`}>
                  {r.job_name}
                </Link>
              ),
            },
            { key: 'state', header: 'State', cell: (r) => <StateBadge state={r.state} label={waitingText(r)} /> },
            { key: 'commit', header: 'Commit', cell: (r) => <span className="font-mono">{short(r.commit)}</span> },
            { key: 'inventory', header: 'Inventory', cell: (r) => r.inventory },
            { key: 'hosts', header: 'Hosts', cell: (r) => <span className="tabular-nums">{r.hosts.length}</span> },
            { key: 'trigger', header: 'Trigger', cell: (r) => r.trigger },
            { key: 'created', header: 'Created', cell: (r) => <span className="tabular-nums">{r.created_at}</span> },
          ]}
          empty={<LoadState loading label="Loading runs…" />}
        />
      )}
    </Page>
  )
}
