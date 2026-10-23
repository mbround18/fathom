/** 027-fleet-rollup-dashboard: read-only grid of every context's health. Backend pollers run
 * only while this view is mounted (Constitution V).
 *
 * 046-fleet-model US2 (T035): the grid is organised by group. A group is a named label selector,
 * so a member appears under every group it matches and nothing is filed per cluster (FR-004,
 * SC-002); a group matching nothing says so rather than vanishing (FR-005); members no group
 * matches stay reachable under an explicit ungrouped listing (FR-017); and the filter states how
 * many of how many members it is showing (FR-016).
 *
 * The rule this view exists to keep: **every aggregate renders with its coverage**. A rollup with
 * `members_covered < members_total` states `n of m`, names each member it could not read, and says
 * which kind of blind spot it was — not permitted, unreachable, absent or not yet polled, four
 * distinct renderings (FR-010, FR-011, SC-004). The defect this avoids is `resources/nav.ts:64-69`,
 * where a *denied* scan renders as "not installed": a conclusion drawn over what could be read,
 * presented as the whole truth. A sum over a subset is never shown here without its subset.
 *
 * 046 US3 (T043) and T051 add the two states where an honest message and a convenient one differ.
 * `FleetModelLoadFailed` says the fleet **file could not be read** and names it, because the
 * convenient rendering — an empty grid, "connect a context" — explains the operator's unread data
 * as data they never had; the backend left the bytes untouched (FR-007, Principle VI). And the
 * sharing panel renders `FleetStoreUnavailable` as "nothing is being followed", never as "nothing
 * is here": an adopted definition survives an unreachable store (FR-014).
 *
 * No new background work: the mount-scoped `start()`/`stop()` pair is unchanged, and a rollup is
 * requested only when the model the backend reported actually changes (FR-015, Principle V).
 */
import { useEffect, useMemo, useState } from 'react'

import { Input } from '@/components/ui/input'
import { cn } from '@companion/theme'
import type { FleetContextSummary, FleetGapReason, FleetMember } from '@/protocol/types'
import {
  coverageLabel,
  coverageState,
  gapDetail,
  gapHeadline,
  orderedGaps,
  partitionFleet,
  showingLabel,
} from '@/resources/fleet'
import { useStore } from '@/store'

import { FleetContextCard } from './FleetContextCard'
import { FleetGroupEditor } from './FleetGroupEditor'
import { FleetSharingPanel } from './FleetSharingPanel'

function pending(context: string): FleetContextSummary {
  return {
    context,
    health: 'Pending',
    error: null,
    plugin_error: null,
    node_ready_count: null,
    node_total_count: null,
    unhealthy_pod_count: null,
    cluster_version: null,
    flux: null,
    restricted: [],
    last_updated: '',
    live: false,
  }
}

/**
 * A context the kubeconfig has but `fleet.json` has nothing to say about yet. With no labels it is
 * in no group, which is exactly what the backend would have resolved for it.
 */
function unnamed(context: string): FleetMember {
  return {
    context,
    display_name: null,
    labels: {},
    origins: {},
    availability: 'Present',
    groups: [],
  }
}

/** One rendering per gap reason — the three the operator confuses at their peril look different. */
const GAP_CLASS: Record<FleetGapReason['kind'], string> = {
  notPermitted: 'border-amber-500/60 bg-amber-500/10 text-amber-300',
  unreachable: 'border-red-500/60 bg-red-500/10 text-red-300',
  absent: 'border-border/60 bg-muted/30 text-muted-foreground',
  pending: 'border-sky-500/50 bg-sky-500/10 text-sky-300',
}

export function FleetDashboardView({ onSelected }: { onSelected?: () => void }) {
  const contexts = useStore((s) => s.session.contexts)
  const summaries = useStore((s) => s.session.fleetSummaries)
  const active = useStore((s) => s.session.activeContext)
  const fleetMembers = useStore((s) => s.session.fleetMembers)
  const groups = useStore((s) => s.session.fleetGroups)
  const rollups = useStore((s) => s.session.fleetRollups)
  const loadFailure = useStore((s) => s.session.fleetLoadFailure)
  const selectContext = useStore((s) => s.selectContext)
  const send = useStore((s) => s.send)
  const start = useStore((s) => s.startFleetPolling)
  const stop = useStore((s) => s.stopFleetPolling)

  const [filter, setFilter] = useState('')
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 5000)
    return () => clearInterval(id)
  }, [])

  useEffect(() => {
    start()
    // Reading `fleet.json` starts no poller, client or watch (FR-015).
    send({ type: 'RefreshFleetModel' })
    return () => stop()
  }, [send, start, stop])

  /**
   * The members the view works over: what the backend reported, plus any kubeconfig context the
   * model has not described yet, so a cluster is never missing from the fleet because nobody has
   * named it. The context name is the key either way (FR-001).
   */
  const members = useMemo(() => {
    const seen = new Set(Object.keys(fleetMembers))
    const extra = contexts.filter((c) => !seen.has(c)).map(unnamed)
    return [...Object.values(fleetMembers), ...extra]
  }, [contexts, fleetMembers])

  // One rollup request per group whenever the reported model changes — membership follows the
  // labels, so a relabelled cluster changes a rollup without the operator doing anything (SC-002).
  useEffect(() => {
    for (const g of groups) send({ type: 'RequestFleetGroupRollup', data: { group: g.name } })
  }, [groups, members, send])

  const view = useMemo(() => partitionFleet(members, groups, filter), [members, groups, filter])

  const card = (member: FleetMember) => (
    <FleetContextCard
      summary={summaries[member.context] ?? pending(member.context)}
      active={member.context === active}
      now={now}
      onSelect={(c) => {
        selectContext(c)
        onSelected?.()
      }}
    />
  )

  /**
   * FR-007, US1 scenario 4: `fleet.json` could not be read, so the backend is running on defaults
   * and **left the file exactly as it was**. Saying "no clusters yet" here would explain an
   * unreadable file as an empty fleet and invite the operator to start over on top of data that is
   * still on disk — the one thing Constitution Principle VI forbids. So the path and the loader's
   * own words are stated, and the file is described as unread, never as discarded.
   */
  const failure =
    loadFailure === null ? null : (
      <p
        data-testid="fleet-load-failure"
        className="flex min-w-0 flex-col gap-1 rounded border border-amber-500/60 bg-amber-500/10 p-2 text-xs text-amber-300"
      >
        <span>
          Your fleet metadata could not be read, so Fathom is working from defaults for now. The
          file is still on disk exactly as it was — it has not been rewritten, and nothing in it has
          been thrown away.
        </span>
        <span className="flex min-w-0 flex-col gap-0.5 font-mono text-muted-foreground">
          <span data-testid="fleet-load-failure-path" className="break-all">
            {loadFailure.path}
          </span>
          {/* The loader's own message, verbatim (Principle I). */}
          <span data-testid="fleet-load-failure-reason" className="break-words">
            {loadFailure.reason}
          </span>
        </span>
      </p>
    )

  if (contexts.length === 0 && members.length === 0) {
    return (
      <div data-testid="fleet-dashboard" className="flex min-w-0 flex-col gap-3 p-2">
        {failure}
        {/* With a load failure on screen, "connect a context" would be a second, false
            explanation of the same emptiness, so only the true one is shown. */}
        {loadFailure === null ? (
          <p data-testid="fleet-empty" className="text-sm text-muted-foreground">
            Connect a context to see fleet health.
          </p>
        ) : null}
        <FleetSharingPanel />
      </div>
    )
  }

  return (
    <div data-testid="fleet-dashboard" className="flex min-w-0 flex-col gap-3 p-2">
      {failure}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
        <Input
          data-testid="fleet-filter"
          aria-label="Filter clusters by name or label"
          placeholder="env=prod"
          className="font-mono sm:max-w-xs"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        {/* FR-016: a filter always says how many of how many it is showing. */}
        <p data-testid="fleet-filter-count" className="text-xs text-muted-foreground">
          {showingLabel(view.shown, view.total)}
        </p>
      </div>

      <FleetGroupEditor />
      {/* US3: publish, adopt and remove — and the two states that must not lie (FR-012/FR-014). */}
      <FleetSharingPanel />

      {view.groups.map(({ group, members: held, readable }) => {
        const rollup = rollups[group.name]
        const coverage =
          rollup === undefined
            ? 'unknown'
            : coverageState(rollup.members_covered, rollup.members_total)
        return (
          <section
            key={group.name}
            data-testid="fleet-group"
            data-group={group.name}
            data-coverage={coverage}
            className="flex min-w-0 flex-col gap-2 rounded-md border border-border/60 p-2"
          >
            <header className="flex flex-wrap items-center gap-2">
              <h3 className="text-sm font-medium">{group.name}</h3>
              <code
                data-testid="fleet-group-selector"
                className="truncate rounded bg-muted/40 px-1 py-0.5 font-mono text-xs text-muted-foreground"
              >
                {group.selector}
              </code>
              <button
                type="button"
                data-testid="fleet-group-delete"
                aria-label={`Delete group ${group.name}`}
                className="ml-auto text-xs text-muted-foreground hover:text-red-400"
                onClick={() => send({ type: 'DeleteFleetGroup', data: { name: group.name } })}
              >
                Delete
              </button>
            </header>

            {/* The sums and the coverage they were taken over, in one sentence (FR-010). */}
            <p data-testid="fleet-group-coverage" className="text-xs text-muted-foreground">
              {rollup === undefined
                ? 'No rollup reported yet — no coverage to state.'
                : `${coverageLabel(rollup.members_covered, rollup.members_total)} read · ` +
                  `${rollup.node_ready} of ${rollup.node_total} nodes ready · ` +
                  `${rollup.unhealthy_pods} unhealthy pods`}
            </p>

            {rollup !== undefined && rollup.gaps.length > 0 ? (
              <ul data-testid="fleet-group-gaps" className="flex flex-col gap-1">
                {orderedGaps(rollup.gaps).map((g) => {
                  const detail = gapDetail(g.reason)
                  return (
                    <li
                      key={g.context}
                      data-testid="fleet-gap"
                      data-group={group.name}
                      data-context-name={g.context}
                      data-reason={g.reason.kind}
                      className="flex min-w-0 flex-col gap-0.5 text-xs"
                    >
                      <span className="flex min-w-0 flex-wrap items-center gap-1.5">
                        <span className="truncate font-mono">{g.context}</span>
                        <span
                          data-testid="fleet-gap-reason"
                          className={cn(
                            'shrink-0 rounded border px-1.5 py-0.5',
                            GAP_CLASS[g.reason.kind],
                          )}
                        >
                          {gapHeadline(g.reason)}
                        </span>
                      </span>
                      {/* The API server's own words, never paraphrased (FR-010, Principle I). */}
                      {detail === null ? null : (
                        <span className="break-words text-muted-foreground">{detail}</span>
                      )}
                    </li>
                  )
                })}
              </ul>
            ) : null}

            {!readable ? (
              // A stored selector that cannot be read is not a selector that matches nothing. The
              // backend's parser is the authority; saying "matches no cluster" here would assert a
              // membership nobody resolved (FR-005, FR-011).
              <p data-testid="fleet-group-unreadable" className="text-xs text-amber-300">
                This group&apos;s selector could not be read, so its membership is unknown — not
                empty. Re-save the group to have the backend parse it again.
                {/* The parser's own words, verbatim (Principle I). */}
                {group.selector_error === null ? null : (
                  <span data-testid="fleet-group-selector-error" className="mt-1 block break-words">
                    {group.selector_error}
                  </span>
                )}
              </p>
            ) : held.length === 0 ? (
              <p data-testid="fleet-group-empty" className="text-xs text-muted-foreground">
                This selector matches no cluster.
              </p>
            ) : (
              <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
                {held.map((m) => (
                  <div
                    key={m.context}
                    data-testid="fleet-group-member"
                    data-group={group.name}
                    data-context-name={m.context}
                    className="flex min-w-0"
                  >
                    {card(m)}
                  </div>
                ))}
              </div>
            )}
          </section>
        )
      })}

      {view.ungrouped.length > 0 ? (
        <section
          data-testid="fleet-ungrouped"
          className="flex min-w-0 flex-col gap-2 rounded-md border border-dashed border-border/60 p-2"
        >
          <header className="flex flex-wrap items-center gap-2">
            <h3 className="text-sm font-medium">Ungrouped</h3>
            {/* FR-017: no member disappears for want of a label. */}
            <span className="text-xs text-muted-foreground">
              {view.ungrouped.length} matching no group
            </span>
          </header>
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
            {view.ungrouped.map((m) => (
              <div
                key={m.context}
                data-testid="fleet-ungrouped-member"
                data-context-name={m.context}
                className="flex min-w-0"
              >
                {card(m)}
              </div>
            ))}
          </div>
        </section>
      ) : null}
    </div>
  )
}
