/**
 * One run (spec 055 US2, T072): its state, the live output in a `LogView`, the summary counts and
 * the per-host results with each host's first failure. A run pinned to an inventory revision that
 * has since been replaced says so. A run that has not ended can be cancelled, previewed first.
 */
import { LogView } from '@companion/terminal'
import { Button, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@companion/ui'
import { useEffect, useMemo, useState } from 'react'

import { Link } from '@/app/Link'
import type { Params } from '@/app/router'
import type { HostStats, RunHostInfo, RunInfo, RunSummary } from '@/protocol/wire'

import { ErrorLine, StateBadge } from './parts'
import { active, reasonText, short, useRuns } from './store'

const STATS: (keyof HostStats)[] = ['ok', 'changed', 'failed', 'unreachable', 'skipped', 'rescued', 'ignored']

function statsText(stats: HostStats | null): string {
  if (!stats) return '—'
  const parts = STATS.filter((k) => stats[k] > 0).map((k) => `${k} ${String(stats[k])}`)
  return parts.length > 0 ? parts.join(' · ') : 'no tasks'
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
    <ul aria-label="Summary" className="flex flex-wrap gap-3 text-xs">
      {counts.map(([k, n]) => (
        <li key={k} data-count={k} className="flex gap-1">
          <span className="text-muted-foreground">{k}</span>
          <span className="font-mono">{n}</span>
        </li>
      ))}
      <li data-count="stats" className="text-muted-foreground">
        {statsText(summary.stats)}
      </li>
    </ul>
  )
}

function Hosts({ hosts }: { hosts: RunHostInfo[] }) {
  return (
    <div className="overflow-x-auto">
      <table aria-label="Hosts" className="w-full text-left text-xs">
        <thead className="text-muted-foreground">
          <tr>
            <th className="py-1">Host</th>
            <th>State</th>
            <th>Results</th>
            <th>First failure</th>
          </tr>
        </thead>
        <tbody>
          {hosts.length === 0 && (
            <tr>
              <td colSpan={4} data-pane-state="empty" className="py-1 text-muted-foreground">
                No host has reported yet.
              </td>
            </tr>
          )}
          {hosts.map((h) => (
            <tr key={h.host} data-host-state={h.state} className="border-t border-border align-top">
              <td className="py-1 font-mono">{h.name}</td>
              <td>
                <StateBadge state={h.state} />
                {h.refusal && <span className="ml-1 text-status-err">({h.refusal})</span>}
              </td>
              <td className="font-mono">{statsText(h.stats)}</td>
              <td>
                {h.first_failure && (
                  <div className="flex flex-col gap-0.5">
                    <span>
                      {h.first_failure.unreachable && <span className="mr-1 text-status-err uppercase">unreachable</span>}
                      <span className="font-mono">{h.first_failure.task}</span>
                      {h.first_failure.rc !== null && (
                        <span className="ml-1 text-muted-foreground">rc {h.first_failure.rc}</span>
                      )}
                    </span>
                    <span className="break-all text-status-err">{h.first_failure.message}</span>
                  </div>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/** The confirm dialog for a pending cancel. */
function CancelDialog() {
  const preview = useRuns((s) => s.preview)
  const confirm = useRuns((s) => s.confirm)
  const cancel = useRuns((s) => s.cancel)
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
            <Button onClick={confirm}>Confirm</Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  )
}

function Header({ run }: { run: RunInfo }) {
  const propose = useRuns((s) => s.propose)
  return (
    <>
      <div className="flex flex-wrap items-center gap-3">
        <h1 id="run-title" className="font-display text-sm font-semibold tracking-wider">
          Run of {run.job_name}
        </h1>
        <span data-run-state={run.state}>
          <StateBadge state={run.state} />
        </span>
        {run.reason && <span className="text-xs text-muted-foreground">{reasonText(run.reason)}</span>}
        {active(run.state) && (
          <Button
            size="xs"
            variant="destructive"
            className="ml-auto"
            onClick={() => propose({ type: 'PreviewCancel', data: { run: run.id } })}
          >
            Cancel run
          </Button>
        )}
      </div>
      {run.newer_inventory_revision && (
        <p role="note" aria-label="Newer inventory" className="border border-border px-2 py-1 text-xs text-muted-foreground">
          This run used an older revision: a newer revision of inventory {run.inventory} exists, and the next run uses
          it.
        </p>
      )}
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt className="text-muted-foreground">Commit</dt>
        <dd className="font-mono">{short(run.commit)}</dd>
        <dt className="text-muted-foreground">Inventory</dt>
        <dd>{run.inventory}</dd>
        <dt className="text-muted-foreground">Trigger</dt>
        <dd>{run.trigger}</dd>
        <dt className="text-muted-foreground">Started</dt>
        <dd>{run.started_at ?? 'not yet'}</dd>
        <dt className="text-muted-foreground">Ended</dt>
        <dd>{run.ended_at ?? '—'}</dd>
      </dl>
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
    <section data-page="run" aria-labelledby="run-title" className="flex flex-col gap-3 p-4">
      <Link to="/runs" className="text-xs text-muted-foreground hover:underline">
        All runs
      </Link>
      <ErrorLine />
      {run ? (
        <Header run={run} />
      ) : (
        <h1 id="run-title" className="font-display text-sm font-semibold tracking-wider">
          Run
        </h1>
      )}
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
        className="max-h-96 min-h-24"
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
    </section>
  )
}
