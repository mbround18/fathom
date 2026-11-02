/**
 * One run (spec 055 US2, T072): its state, the live output in a `LogView`, the summary counts and
 * the per-host results with each host’s first failure. The job params it was queued with sit next to
 * its commit (spec 061). A run pinned to an inventory revision that has since been replaced says so.
 * A run that has not ended can be cancelled, previewed first. A run held for approval (spec 064) shows
 * its approval panel above the output.
 */
import { ArrowLeftIcon, StopIcon } from '@phosphor-icons/react'
import { LogView } from '@companion/terminal'
import { Button } from '@companion/ui'
import { ConfirmDialog, DataTable, H2, LABEL, Link, LINK, Page, type Params } from '@companion/ui/kit'
import { useEffect, useMemo, useState } from 'react'

import { useSession } from '@/app/session-store'
import { canOperate } from '@/features/hosts/workspaces'
import type { HostStats, RunHostInfo, RunInfo, RunSummary } from '@/protocol/wire'
import { when } from '@/features/settings/updates'
import { PageHeader } from '@/ui/PageHeader'

import { ApprovalPanel, DecisionDialog } from './ApprovalPanel'
import { ErrorLine, StateBadge } from './parts'
import { active, reasonText, short, useRuns } from './store'

const STATS: (keyof HostStats)[] = ['ok', 'changed', 'failed', 'unreachable', 'skipped', 'rescued', 'ignored']

function statsText(stats: HostStats | null): string {
  if (!stats) return '—'
  const parts = STATS.filter((k) => stats[k] > 0).map((k) => `${k} ${String(stats[k])}`)
  return parts.length > 0 ? parts.join(' · ') : 'no tasks'
}

const TONE: Record<string, string> = {
  succeeded: 'text-status-ok',
  failed: 'text-status-err',
  refused: 'text-status-err',
  lost: 'text-status-err',
}

/** One job param the run was queued with (spec 061 FR-019), e.g. the image digest an update moves to. */
function Param({ name, value }: { name: string; value: string }) {
  return (
    <>
      <dt className="min-w-0 font-mono break-all text-muted-foreground">{name}</dt>
      <dd data-param={name} title={value} className="min-w-0 font-mono break-all">
        {value}
      </dd>
    </>
  )
}

function Summary({ summary }: { summary: RunSummary }) {
  const counts: [string, number][] = [
    ['hosts', summary.hosts],
    ['succeeded', summary.succeeded],
    ['failed', summary.failed],
    ['refused', summary.refused],
    ['lost', summary.lost],
    ['cancelled', summary.cancelled],
  ]
  return (
    <ul aria-label="Summary" className="flex min-w-0 flex-wrap gap-2 text-xs">
      {counts.map(([k, n]) => (
        <li
          key={k}
          data-count={k}
          className={`flex min-w-20 flex-col border border-border px-2 py-1 ${n > 0 && TONE[k] ? TONE[k] : ''}`}
        >
          <span className={LABEL}>{k}</span>
          <span className="font-mono text-sm tabular-nums">{n}</span>
        </li>
      ))}
      <li data-count="stats" className="flex items-center px-1 text-muted-foreground">
        {statsText(summary.stats)}
      </li>
    </ul>
  )
}

function Hosts({ hosts }: { hosts: RunHostInfo[] }) {
  return (
    <section aria-labelledby="run-hosts-title" className="flex min-w-0 flex-col gap-2">
      <h2 id="run-hosts-title" className={H2}>
        Hosts
      </h2>
      <DataTable<RunHostInfo>
        label="Hosts"
        rows={hosts}
        rowKey={(h) => h.host}
        rowData={(h) => ({ 'data-host-state': h.state })}
        columns={[
          { key: 'host', header: 'Host', className: 'w-[20%]', cell: (h) => <span className="font-mono break-all">{h.name}</span> },
          {
            key: 'state',
            header: 'State',
            cell: (h) => (
              <>
                <StateBadge state={h.state} />
                {h.refusal && <span className="ml-1 text-status-err">({h.refusal})</span>}
              </>
            ),
          },
          { key: 'results', header: 'Results', cell: (h) => <span className="font-mono">{statsText(h.stats)}</span> },
          {
            key: 'failure',
            header: 'First failure',
            className: 'w-[36%]',
            cell: (h) =>
              h.first_failure && (
                <div className="flex min-w-0 flex-col gap-0.5">
                  <span>
                    {h.first_failure.unreachable && <span className="mr-1 font-medium text-status-err">Unreachable</span>}
                    <span className="font-mono break-all">{h.first_failure.task}</span>
                    {h.first_failure.rc !== null && <span className="ml-1 text-muted-foreground">rc {h.first_failure.rc}</span>}
                  </span>
                  <span className="break-all text-status-err">{h.first_failure.message}</span>
                </div>
              ),
          },
        ]}
        empty="No host has reported yet."
      />
    </section>
  )
}


/** The confirm dialog for a pending cancel. */
function CancelDialog() {
  const preview = useRuns((s) => s.preview)
  const confirm = useRuns((s) => s.confirm)
  const cancel = useRuns((s) => s.cancel)
  return <ConfirmDialog preview={preview} title="Cancel this run" tone="destructive" onCancel={cancel} onConfirm={confirm} />
}

function Header({ run }: { run: RunInfo }) {
  const propose = useRuns((s) => s.propose)
  const me = useSession((s) => s.me)
  return (
    <>
      <PageHeader
        id="run-title"
        title={`Run of ${run.job_name}`}
        description={
          <span className="flex flex-wrap items-center gap-2">
            <span data-run-state={run.state}>
              <StateBadge state={run.state} />
            </span>
            {run.reason && <span>{reasonText(run.reason)}</span>}
          </span>
        }
        actions={
          // Cancelling takes Operate in the run's workspace; a viewer would only meet a refusal.
          active(run.state) &&
          canOperate(me, run.workspace) && (
            <Button size="sm" variant="destructive" onClick={() => propose({ type: 'PreviewCancel', data: { run: run.id } })}>
              <StopIcon aria-hidden />
              Cancel run
            </Button>
          )
        }
      />
      {run.newer_inventory_revision && (
        <p role="note" aria-label="Newer inventory" className="border border-status-warn/50 px-3 py-2 text-xs text-status-warn">
          This run used an older revision: a newer revision of inventory {run.inventory} exists, and the next run uses
          it.
        </p>
      )}
      <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-xs sm:grid-cols-[auto_minmax(0,1fr)_auto_minmax(0,1fr)]">
        <dt className="text-muted-foreground">Commit</dt>
        <dd className="font-mono">{short(run.commit)}</dd>
        <dt className="text-muted-foreground">Inventory</dt>
        <dd>{run.inventory}</dd>
        <dt className="text-muted-foreground">Trigger</dt>
        <dd>{run.trigger}</dd>
        <dt className="text-muted-foreground">Started</dt>
        <dd className="tabular-nums">{run.started_at ? when(run.started_at) : 'not yet'}</dd>
        <dt className="text-muted-foreground">Ended</dt>
        <dd className="tabular-nums">{run.ended_at ? when(run.ended_at) : '—'}</dd>
      </dl>
      {/* Params get their own list: a name like keel_server_image_digest and a full digest would
          otherwise widen the label column of every field above. On a phone the value sits under its
          name. */}
      {Object.keys(run.params).length > 0 && (
        <dl aria-label="Parameters" className="grid min-w-0 grid-cols-1 gap-x-4 gap-y-0.5 text-xs sm:grid-cols-[auto_minmax(0,1fr)] sm:gap-y-1">
          {Object.entries(run.params)
            .sort(([a], [b]) => a.localeCompare(b))
            .map(([name, value]) => (
              <Param key={name} name={name} value={value} />
            ))}
        </dl>
      )}
    </>
  )
}

export default function RunPage({ params }: { params: Params }) {
  const id = params.id ?? null
  const watch = useRuns((s) => s.watch)
  const detail = useRuns((s) => s.detail)
  const lines = useRuns((s) => s.lines)
  const partial = useRuns((s) => s.partial)
  const dropped = useRuns((s) => s.dropped)
  const [follow, setFollow] = useState(true)

  useEffect(() => {
    watch(id)
    return () => {
      watch(null)
    }
  }, [id, watch])

  const shown = useMemo(() => (partial ? [...lines, partial] : lines), [lines, partial])
  const run = detail?.run.id === id ? detail.run : null

  return (
    <Page name="run" labelledBy="run-title">
      <Link to="/runs" className={`${LINK} flex w-fit items-center gap-1 text-xs text-muted-foreground`}>
        <ArrowLeftIcon aria-hidden className="size-3.5" />
        All runs
      </Link>
      <ErrorLine />
      {run ? <Header run={run} /> : <PageHeader id="run-title" title="Run" description="Loading the run…" />}
      {run && <ApprovalPanel run={run} />}
      {run?.summary && <Summary summary={run.summary} />}
      <LogView
        role="log"
        aria-label="Run output"
        aria-live="polite"
        lines={shown}
        follow={follow}
        wrap
        dropped={dropped}
        onFollowChange={setFollow}
        className="max-h-96 min-h-24 border border-border"
        placeholder={
          shown.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              {run && !active(run.state) ? 'No output.' : 'Waiting for output…'}
            </p>
          ) : undefined
        }
      />
      {run && <Hosts hosts={detail?.hosts ?? []} />}
      <CancelDialog />
      <DecisionDialog />
    </Page>
  )
}
