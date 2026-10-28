/**
 * Runs (spec 055 US2, T072): the workspace's runs, newest first. Each opens its own page
 * (./RunPage.tsx) with its live output and per-host results.
 */
import { Link } from '@/app/Link'
import { GrantWorkspacePicker } from '@/app/WorkspacePicker'
import type { RunInfo } from '@/protocol/wire'
import { DataTable } from '@/ui/DataTable'
import { LINK } from '@/ui/form'
import { Page } from '@/ui/Page'
import { PageHeader } from '@/ui/PageHeader'

import { ErrorLine, StateBadge } from './parts'
import { short, useRuns } from './store'

export default function RunsPage() {
  const runs = useRuns((s) => s.runs)
  const workspace = useRuns((s) => s.workspace)
  const load = useRuns((s) => s.load)

  return (
    <Page name="runs" labelledBy="runs-title">
      <PageHeader id="runs-title" title="Runs" description="Every job run in the workspace, newest first, with its live output.">
        <GrantWorkspacePicker workspace={workspace} load={load} />
      </PageHeader>
      <ErrorLine />
      {workspace && (
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
          empty="No runs yet. A push to a bound branch starts one."
        />
      )}
    </Page>
  )
}
