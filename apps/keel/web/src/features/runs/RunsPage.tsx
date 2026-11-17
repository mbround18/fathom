/**
 * Runs (spec 055 US2, T072): the workspace's runs, newest first. Each opens its own page
 * (./RunPage.tsx) with its live output and per-host results. "Waiting for approval" (spec 064 US4)
 * narrows the list to held runs, oldest first, and is kept in `?state=pending_approval`.
 *
 * Spec 068: a converge lists once, as its parent run; its steps' child runs show only with "Show
 * steps" (`?steps=1`). Check runs carry a `check` badge.
 */
import { Badge, Button, Checkbox } from '@companion/ui'
import {
  DataTable,
  EmptyState,
  emptyVariant,
  Link,
  LINK,
  LoadFailed,
  LoadState,
  navigate,
  NoMatch,
  Page,
  PageHeader,
  useSearchParam,
} from '@companion/ui/kit'
import { useCallback, useId, useMemo } from 'react'

import { useSession } from '@/app/session-store'
import { CurrentWorkspace } from '@/app/WorkspacePicker'
import { canOperate } from '@/features/hosts/workspaces'
import type { RunInfo } from '@/protocol/wire'

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

/** "Show steps": a converge's child runs, hidden by default (spec 068). */
function StepsToggle({ on, held }: { on: boolean; held: boolean }) {
  const id = useId()
  return (
    <div className="flex items-center gap-2 text-xs">
      <Checkbox
        id={id}
        checked={on}
        onCheckedChange={(c) => {
          const q = [held ? 'state=pending_approval' : null, c === true ? 'steps=1' : null].filter((x) => x !== null)
          navigate(q.length > 0 ? `/runs?${q.join('&')}` : '/runs', { replace: true })
        }}
      />
      <label htmlFor={id}>Show converge steps</label>
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
  const steps = useSearchParam('steps') === '1'
  // The page reloads whenever `load` changes, so a new filter lists again.
  const load = useCallback((ws: string) => loadRuns(ws, held ? 'pending_approval' : null), [loadRuns, held])
  // A run approved while the filter is on stays in the store until the next list: hide it here.
  const shown = useMemo(() => {
    const listed = steps ? runs : runs.filter((r) => r.parent_run === null)
    return held ? oldestFirst(listed) : listed
  }, [held, steps, runs])
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
        <CurrentWorkspace load={load} />
      </PageHeader>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <RunsFilter value={filter} />
        <StepsToggle on={steps} held={held} />
      </div>
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
          rowData={(r) => ({ 'data-run-state': r.state, 'data-run-kind': r.kind })}
          columns={[
            {
              key: 'job',
              header: 'Job',
              className: 'w-[22%]',
              cell: (r) => (
                <span className="flex min-w-0 flex-wrap items-center gap-1">
                  <Link to={`/runs/${r.id}`} className={`${LINK} font-mono break-all`}>
                    {r.kind === 'converge' ? 'converge' : r.job_name}
                  </Link>
                  {r.check && (
                    <Badge variant="outline" data-check-badge>
                      check
                    </Badge>
                  )}
                </span>
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
