/**
 * The converge preview (spec 068 contracts/ui.md "Converge preview", T026): what Confirm would run,
 * on which hosts, under which policy. Rows are hosts, columns are step positions; under 640 px each
 * host is a card with its steps as an ordered list. Hosts refused a step are listed with the
 * reason and left out of Confirm. Params and credential names show per job; values never do.
 *
 * In signed mode each step's commit is reviewed on the review page: one passkey assertion per step,
 * all of them listed there together. Confirm here carries no step-up.
 */
import { Badge, Button, Switch, Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@companion/ui'
import { Dialog, H3, LABEL, Link, LINK, LoadState } from '@companion/ui/kit'
import { useId } from 'react'

import { useRepos } from '@/features/bindings/store'
import { short } from '@/features/runs/store'
import type { ConvergePlan, HostPlan, PlannedStep } from '@/protocol/wire'

import { confirmLabel, jobsOf, planCounts, planMatrix, policyText, refusalText, viaText } from './derive'
import { useConverge } from './store'

function Step({ step }: { step: PlannedStep }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* A button so the "via" chain shows on focus for the keyboard too. */}
        <button
          type="button"
          data-step={step.job}
          className="text-left font-mono break-all focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none"
        >
          {step.job}
        </button>
      </TooltipTrigger>
      <TooltipContent>{viaText(step)}</TooltipContent>
    </Tooltip>
  )
}

function Refusals({ host }: { host: HostPlan }) {
  if (host.refusals.length === 0) return null
  return (
    <ul aria-label={`Why ${host.host} is left out`} className="text-xs text-status-err">
      {host.refusals.map((r) => (
        <li key={`${r.job}-${r.reason}`}>{refusalText(r.job, r.reason)}</li>
      ))}
    </ul>
  )
}

/** ≥ 640 px: the matrix. */
function Matrix({ plan }: { plan: ConvergePlan }) {
  const { positions, rows } = planMatrix(plan)
  return (
    <div className="hidden overflow-x-auto sm:block">
      <table aria-label="Plan" className="w-full text-xs">
        <thead>
          <tr className="border-b border-border text-left">
            <th scope="col" className={`${LABEL} py-1 pr-3`}>
              Host
            </th>
            {positions.map((p) => (
              <th key={p} scope="col" className={`${LABEL} py-1 pr-3 tabular-nums`}>
                Step {p}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr
              key={row.host.host_id}
              data-plan-host={row.host.host}
              data-included={row.included}
              className="border-b border-border/50 align-top"
            >
              <th scope="row" className="py-1.5 pr-3 text-left font-medium">
                <span className={row.included ? '' : 'text-muted-foreground line-through'}>{row.host.host}</span>
                <Refusals host={row.host} />
              </th>
              {row.cells.map((cell, i) => (
                <td key={positions[i]} className="py-1.5 pr-3">
                  {cell ? <Step step={cell} /> : <span className="text-muted-foreground">—</span>}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/** < 640 px: one card per host. */
function Cards({ plan }: { plan: ConvergePlan }) {
  return (
    <ul aria-label="Plan" className="flex flex-col gap-2 sm:hidden">
      {plan.hosts.map((host) => {
        const included = host.steps.length > 0 && host.refusals.length === 0
        return (
          <li key={host.host_id} data-plan-host={host.host} data-included={included} className="border border-border p-2">
            <p className={`${H3} ${included ? '' : 'text-muted-foreground line-through'}`}>{host.host}</p>
            <Refusals host={host} />
            {host.steps.length > 0 && (
              <ol aria-label={`Steps on ${host.host}`} className="mt-1 list-decimal pl-5 text-xs">
                {host.steps.map((s) => (
                  <li key={s.position}>
                    <span className="font-mono break-all">{s.job}</span>{' '}
                    <span className="text-muted-foreground">{viaText(s)}</span>
                  </li>
                ))}
              </ol>
            )}
          </li>
        )
      })}
    </ul>
  )
}

function Jobs({ plan }: { plan: ConvergePlan }) {
  const jobs = jobsOf(plan)
  if (jobs.length === 0) return null
  return (
    <section aria-label="Steps" className="flex flex-col gap-1">
      <h3 className={H3}>Steps</h3>
      <ul className="flex flex-col gap-1.5 text-xs">
        {jobs.map((j) => (
          <li key={j.job} data-job={j.job} className="min-w-0">
            <span className="font-mono break-all">{j.job}</span>{' '}
            <span className="text-muted-foreground">
              on {j.hosts} {j.hosts === 1 ? 'host' : 'hosts'}
            </span>
            {j.params.length > 0 && (
              <dl aria-label={`Parameters of ${j.job}`} className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 pl-3">
                {j.params.map(([k, v]) => (
                  <div key={k} className="contents">
                    <dt className="font-mono text-muted-foreground">{k}</dt>
                    <dd className="font-mono break-all">{v}</dd>
                  </div>
                ))}
              </dl>
            )}
            {j.credential_names.length > 0 && (
              <p className="pl-3">
                <span className="text-muted-foreground">Credentials: </span>
                <span data-credentials className="font-mono">
                  {j.credential_names.join(', ')}
                </span>
              </p>
            )}
          </li>
        ))}
      </ul>
    </section>
  )
}

/** Signed mode: each step's commit, grouped by child run, reviewed on the review page. */
function Review({ plan }: { plan: ConvergePlan }) {
  if (plan.review.length === 0) return null
  const pending = plan.review.filter((r) => r.needs_review).length
  return (
    <section aria-label="Review" className="flex flex-col gap-1">
      <h3 className={H3}>Review</h3>
      <p className="text-xs text-muted-foreground">
        {pending === 0
          ? 'Every step’s commit is reviewed.'
          : `${String(pending)} of ${String(plan.review.length)} steps need a reviewed commit: one passkey assertion per step, signed together on the review page.`}
      </p>
      <ul className="text-xs">
        {plan.review.map((r) => (
          <li key={`${r.job}-${r.commit}`} data-review={r.job} data-needs-review={r.needs_review} className="flex flex-wrap gap-x-2">
            <span className="font-mono">{r.job}</span>
            <span className="font-mono text-muted-foreground">{short(r.commit)}</span>
            <span className={r.needs_review ? 'text-status-warn' : 'text-status-ok'}>
              {r.needs_review ? 'needs review' : 'reviewed'}
            </span>
          </li>
        ))}
      </ul>
      {pending > 0 && (
        <Link to="/reviews" className={`${LINK} w-fit text-xs`}>
          Open the review page
        </Link>
      )}
    </section>
  )
}

function Notes({ plan }: { plan: ConvergePlan }) {
  if (plan.replaces.length === 0 && plan.blocked_by.length === 0) return null
  return (
    <ul aria-label="Notes" className="flex flex-col gap-1 text-xs text-status-warn">
      {plan.replaces.map((id) => (
        <li key={id} data-replaces={id}>
          Replaces an unapproved converge (
          <Link to={`/runs/${id}`} className={LINK}>
            {short(id)}
          </Link>
          ).
        </li>
      ))}
      {plan.blocked_by.map((id) => (
        <li key={id} data-blocked-by={id}>
          A host already has a converge running (
          <Link to={`/runs/${id}`} className={LINK}>
            {short(id)}
          </Link>
          ); it is left out.
        </li>
      ))}
    </ul>
  )
}

function Header({ plan }: { plan: ConvergePlan }) {
  const binding = useRepos((s) => s.bindings.find((b) => b.id === plan.binding_id))
  return (
    <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
      <dt className="text-muted-foreground">Binding</dt>
      <dd className="break-all">{binding ? `${binding.url} (${binding.branch})` : plan.binding_id}</dd>
      <dt className="text-muted-foreground">Commit</dt>
      <dd className="font-mono">{short(plan.commit)}</dd>
      <dt className="text-muted-foreground">Inventory</dt>
      <dd className="tabular-nums">revision {plan.inventory_revision}</dd>
    </dl>
  )
}

export function ConvergePreview() {
  const asked = useConverge((s) => s.asked)
  const plan = useConverge((s) => s.plan)
  const error = useConverge((s) => s.error)
  const confirming = useConverge((s) => s.confirming)
  const confirm = useConverge((s) => s.confirm)
  const close = useConverge((s) => s.close)
  const setCheck = useConverge((s) => s.setCheck)
  const checkId = useId()
  const retry = asked?.kind === 'retry'
  const check = asked?.kind === 'target' ? asked.target.check : (plan?.check ?? false)
  const counts = plan ? planCounts(plan) : null

  return (
    <Dialog
      open={asked !== null}
      onClose={close}
      size="lg"
      title={retry ? 'Retry failed steps' : 'Converge'}
      description={
        plan ? policyText(plan.policy, new Set(plan.hosts.flatMap((h) => h.steps.map((s) => s.job))).size) : 'Planning…'
      }
      data-converge-preview={plan ? 'plan' : 'loading'}
      footer={
        <>
          <Button variant="outline" onClick={close}>
            Cancel
          </Button>
          <Button
            data-converge-confirm
            disabled={!plan || confirming || (counts?.hosts ?? 0) === 0}
            onClick={confirm}
          >
            {plan ? confirmLabel(plan) : 'Converge'}
          </Button>
        </>
      }
    >
      <TooltipProvider>
        {asked?.kind === 'target' && (
          <div className="flex items-center gap-2 text-xs">
            <Switch
              id={checkId}
              checked={check}
              onCheckedChange={(on) => {
                setCheck(on)
              }}
            />
            <label htmlFor={checkId}>Check only (dry run, changes nothing)</label>
          </div>
        )}
        {error && (
          <p role="alert" className="text-xs text-status-err">
            {error}
          </p>
        )}
        {!plan && !error && <LoadState loading label="Planning the converge…" />}
        {plan && (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <Header plan={plan} />
              {plan.check && (
                <Badge variant="outline" data-check-badge>
                  check
                </Badge>
              )}
            </div>
            <Notes plan={plan} />
            <Matrix plan={plan} />
            <Cards plan={plan} />
            <Jobs plan={plan} />
            <Review plan={plan} />
          </>
        )}
      </TooltipProvider>
    </Dialog>
  )
}
