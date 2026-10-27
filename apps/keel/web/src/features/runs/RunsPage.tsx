/**
 * Runs (spec 055 US2, T072): the workspace's runs, newest first. Each opens its own page
 * (./RunPage.tsx) with its live output and per-host results.
 */
import { useEffect, useId } from 'react'

import { Link } from '@/app/Link'
import { useSession } from '@/app/session-store'

import { ErrorLine, StateBadge } from './parts'
import { FIELD, short, useRuns } from './store'

/** Picks one of the signed-in user's workspaces and lists its runs. */
function WorkspacePicker() {
  const grants = useSession((s) => s.me?.grants ?? [])
  const workspace = useRuns((s) => s.workspace)
  const load = useRuns((s) => s.load)
  const id = useId()
  const first = grants[0]?.workspace ?? null
  const unique = [...new Map(grants.map((g) => [g.workspace, g.workspace_slug])).entries()]

  useEffect(() => {
    const current = workspace ?? first
    if (current) load(current)
  }, [workspace, first, load])

  if (unique.length === 0)
    return (
      <p data-pane-state="empty" className="text-xs text-muted-foreground">
        You have no workspace yet. An owner or admin grants one from Teams.
      </p>
    )
  return (
    <div className="flex items-center gap-2">
      <label htmlFor={id} className="text-[11px] tracking-wider text-muted-foreground uppercase">
        Workspace
      </label>
      <select
        id={id}
        className={FIELD}
        value={workspace ?? first ?? ''}
        onChange={(e) => {
          load(e.target.value)
        }}
      >
        {unique.map(([ws, slug]) => (
          <option key={ws} value={ws}>
            {slug}
          </option>
        ))}
      </select>
    </div>
  )
}

export default function RunsPage() {
  const runs = useRuns((s) => s.runs)
  const workspace = useRuns((s) => s.workspace)

  return (
    <section data-page="runs" aria-labelledby="runs-title" className="flex flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 id="runs-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Runs
        </h1>
        <WorkspacePicker />
      </div>
      <ErrorLine />
      {workspace && (
        <div className="overflow-x-auto">
          <table aria-label="Runs" className="w-full text-left text-xs">
            <thead className="text-muted-foreground">
              <tr>
                <th className="py-1">Job</th>
                <th>State</th>
                <th>Commit</th>
                <th>Inventory</th>
                <th>Hosts</th>
                <th>Trigger</th>
                <th>Created</th>
              </tr>
            </thead>
            <tbody>
              {runs.length === 0 && (
                <tr>
                  <td colSpan={7} data-pane-state="empty" className="py-1 text-muted-foreground">
                    No runs yet. A push to a bound branch starts one.
                  </td>
                </tr>
              )}
              {runs.map((r) => (
                <tr key={r.id} data-run-state={r.state} className="border-t border-border">
                  <td className="py-1">
                    <Link to={`/runs/${r.id}`} className="font-mono hover:underline">
                      {r.job_name}
                    </Link>
                  </td>
                  <td>
                    <StateBadge state={r.state} />
                  </td>
                  <td className="font-mono">{short(r.commit)}</td>
                  <td>{r.inventory}</td>
                  <td>{r.hosts.length}</td>
                  <td>{r.trigger}</td>
                  <td className="whitespace-nowrap">{r.created_at}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  )
}
