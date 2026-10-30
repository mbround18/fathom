/**
 * Runs (spec 055 US2, T072): the workspace's runs, newest first. Each opens its own page
 * (./RunPage.tsx) with its live output and per-host results.
 */
import { Button } from '@companion/ui'

import { Link } from '@/app/Link'
import { useSession } from '@/app/session-store'
import { GrantWorkspacePicker } from '@/app/WorkspacePicker'
import { canOperate } from '@/features/hosts/workspaces'
import type { RunInfo } from '@/protocol/wire'
import { DataTable } from '@/ui/DataTable'
import { EmptyState } from '@/ui/EmptyState'
import { emptyVariant } from '@/ui/empty-variant'
import { LINK } from '@/ui/form'
import { LoadState } from '@/ui/LoadState'
import { Page } from '@/ui/Page'
import { PageHeader } from '@/ui/PageHeader'

import { ErrorLine, StateBadge } from './parts'
import { short, useRuns } from './store'

/** No runs in the workspace yet (spec 063 FR-011, FR-012). */
function NoRuns({ canAct }: { canAct: boolean }) {
  return (
    <EmptyState
      page="runs"
      title="No runs yet"
      reason="A run applies a bound branch to its hosts."
      steps={['Bind a repository.', 'Push to the bound branch.', 'Approve the review if Keel asks.']}
      action={
        canAct ? (
          <Button asChild size="sm">
            <Link to="/bindings">Go to bindings</Link>
          </Button>
        ) : undefined
      }
      whoCan="An operator or workspace admin"
      verb="bind repositories"
      guide={{ guide: 'getting-started', anchor: 'your-first-run' }}
    />
  )
}

export default function RunsPage() {
  const runs = useRuns((s) => s.runs)
  const loaded = useRuns((s) => s.loaded)
  const workspace = useRuns((s) => s.workspace)
  const load = useRuns((s) => s.load)
  const me = useSession((s) => s.me)
  const variant = emptyVariant({ filtered: false, canAct: workspace !== null && canOperate(me, workspace) })

  return (
    <Page name="runs" labelledBy="runs-title">
      <PageHeader id="runs-title" title="Runs" description="Every job run in the workspace, newest first, with its live output.">
        <GrantWorkspacePicker workspace={workspace} load={load} />
      </PageHeader>
      <ErrorLine />
      {workspace && loaded && runs.length === 0 && <NoRuns canAct={variant === 'guided'} />}
      {workspace && !(loaded && runs.length === 0) && (
        <DataTable<RunInfo>
          label="Runs"
          rows={runs}
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
            { key: 'state', header: 'State', cell: (r) => <StateBadge state={r.state} /> },
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
