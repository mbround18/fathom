/**
 * Repository → Jobs (spec 066 T051): the jobs `.keel/jobs.yaml` declares on the primary binding,
 * what triggers them, and Run for an operator (`PreviewRunStart`, through governance).
 */
import { PlayIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { ChipList, DataTable, LoadFailed, LoadState, type Column } from '@companion/ui/kit'
import { useMemo } from 'react'

import { useSession } from '@/app/session-store'
import { canOperate } from '@/features/hosts/workspaces'
import { useGovernance } from '@/features/teams/store'
import type { JobInfo } from '@/protocol/wire'

import { useSelected } from './address'
import { DetailPane, Flag, ItemButton, ItemLink, Lines, NothingFound, Part, Split } from './parts'
import { useCatalog } from './store'

function Trigger({ job }: { job: JobInfo }) {
  return job.paths === null ? <span className="text-muted-foreground">manual</span> : <span className="font-mono break-all">{job.paths.join(', ')}</span>
}

function Detail({ job, canRun }: { job: JobInfo; canRun: boolean }) {
  const propose = useGovernance((s) => s.propose)
  return (
    <DetailPane
      label="Job"
      title={job.name}
      badges={job.blocked && <Flag tone="err">blocked</Flag>}
      actions={
        canRun &&
        !job.blocked && (
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              propose(`Run ${job.name}`, { type: 'PreviewRunStart', data: { job: job.id, limit: null } })
            }}
          >
            <PlayIcon aria-hidden />
            Run
          </Button>
        )
      }
    >
      {job.blocked && (
        <p role="alert" className="break-words text-status-err">
          {job.blocked}
        </p>
      )}
      <Part title="Playbook">
        <ItemLink tab="playbooks" item={job.playbook} />
      </Part>
      <Part title="Profiles">
        <ChipList label="Profiles" items={job.profiles} />
      </Part>
      <Part title="Inventory">
        <code className="font-mono break-all">
          {job.inventory}
          {job.limit ? ` --limit ${job.limit}` : ''}
        </code>
      </Part>
      <Part title="Runs on push to">
        <Trigger job={job} />
      </Part>
      <Part title="Credentials">
        <Lines label="Credentials" items={job.credentials} />
      </Part>
      <Part title="Declared at">
        <code className="font-mono">{job.defined_at_commit.slice(0, 12)}</code>
      </Part>
    </DetailPane>
  )
}

export default function JobsTab({ workspace }: { workspace: string }) {
  const selected = useSelected()
  const jobs = useCatalog((s) => s.jobs)
  const failed = useCatalog((s) => s.jobsFailed)
  const binding = useCatalog((s) => s.binding)
  const load = useCatalog((s) => s.load)
  const me = useSession((s) => s.me)
  const columns = useMemo<Column<JobInfo>[]>(
    () => [
      {
        key: 'name',
        header: 'Job',
        className: 'w-[25%]',
        cell: (j) => (
          <span className="flex min-w-0 flex-wrap items-center gap-1">
            <ItemButton item={j.name}>{j.name}</ItemButton>
            {j.blocked && <Flag tone="err">blocked</Flag>}
          </span>
        ),
      },
      { key: 'playbook', header: 'Playbook', cell: (j) => <ItemLink tab="playbooks" item={j.playbook} /> },
      { key: 'profiles', header: 'Profiles', cell: (j) => <ChipList label={`Profiles of ${j.name}`} items={j.profiles} /> },
      { key: 'trigger', header: 'Trigger', cell: (j) => <Trigger job={j} /> },
    ],
    [],
  )
  if (failed) {
    return (
      <LoadFailed
        what="jobs"
        refusal={failed}
        onRetry={() => {
          load({ id: workspace, primary_binding: binding })
        }}
      />
    )
  }
  if (jobs === null) return <LoadState loading label="Loading jobs…" />
  if (jobs.length === 0) return <NothingFound what="jobs" reason="Jobs are declared in .keel/jobs.yaml. Draft one from a playbook." />
  const open = jobs.find((j) => j.name === selected)
  return (
    <Split>
      <DataTable
        label="Jobs"
        columns={columns}
        rows={jobs}
        rowKey={(j) => j.id}
        rowData={(j) => ({ 'data-testid': 'catalog-item', 'data-item': j.name })}
        selected={(j) => j.name === selected}
        empty={null}
      />
      {open && <Detail job={open} canRun={canOperate(me, workspace)} />}
    </Split>
  )
}
