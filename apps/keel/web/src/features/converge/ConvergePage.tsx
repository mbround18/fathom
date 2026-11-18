/**
 * A converge run (spec 068 contracts/ui.md "Converge page", T027): the plan's matrix, live. Rows
 * are hosts, columns are step positions; each cell is the step's state with an icon and a text
 * label (read out always, shown in the cell at ≥ 1024 px) and opens the step's child run for that
 * host. Approve and Deny are the run page's approval panel on the parent. Cancel stops the steps
 * not yet sent; Retry failed previews only the steps that did not succeed, optionally re-planned at
 * the binding's latest commit.
 *
 * RunPage renders this in place of the plain run view when the run's kind is `converge`.
 */
import { ArrowClockwiseIcon, ArrowLeftIcon, StopIcon } from '@phosphor-icons/react'
import { Button, Checkbox } from '@companion/ui'
import { H2, LABEL, Link, LINK, LoadState, Page, PageHeader } from '@companion/ui/kit'
import { useEffect, useId, useState } from 'react'

import { useSession } from '@/app/session-store'
import { canOperate } from '@/features/hosts/workspaces'
import { ApprovalPanel, DecisionDialog } from '@/features/runs/ApprovalPanel'
import { CheckBadge, RunChecks } from '@/features/runs/checks'
import { ErrorLine, StateBadge } from '@/features/runs/parts'
import { active, short } from '@/features/runs/store'
import type { ConvergeStepView, ConvergeView, RunInfo } from '@/protocol/wire'

import { ConvergePreview } from './ConvergePreview'
import { finished, retryableSteps, stepLabel, toneClass, viewMatrix } from './derive'
import { StepIcon } from './StepIcon'
import { useConverge } from './store'

function Cell({ step, host }: { step: ConvergeStepView; host: string }) {
  const label = stepLabel(step.state)
  return (
    <Link
      to={`/runs/${step.run_id}?host=${encodeURIComponent(host)}`}
      data-step-state={step.state}
      data-step={step.job}
      aria-label={`${step.job} on ${host}: ${label}`}
      className={`${LINK} flex min-w-0 items-center gap-1.5 no-underline`}
    >
      <StepIcon state={step.state} />
      <span className="flex min-w-0 flex-col">
        <span className={`font-mono break-all text-foreground ${step.state === 'skipped' ? 'line-through' : ''}`}>
          {step.job}
        </span>
        <span aria-hidden className={`hidden text-[11px] lg:inline ${toneClass(step.state)}`}>
          {label}
        </span>
      </span>
    </Link>
  )
}

function Matrix({ view }: { view: ConvergeView }) {
  const { positions, rows } = viewMatrix(view)
  return (
    <section aria-labelledby="converge-matrix" className="flex flex-col gap-2">
      <h2 id="converge-matrix" className={H2}>
        Hosts and steps
      </h2>
      {/* ≥ 640 px: the matrix. */}
      <div className="hidden overflow-x-auto sm:block">
        <table aria-label="Converge" className="w-full text-xs">
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
              <tr key={row.host_id} data-converge-host={row.host} data-host-state={row.state} className="border-b border-border/50 align-top">
                <th scope="row" className="py-1.5 pr-3 text-left font-medium">
                  {row.host}
                  <span className="block text-[11px] font-normal text-muted-foreground first-letter:uppercase">{row.state}</span>
                </th>
                {row.cells.map((cell, i) => (
                  <td key={positions[i]} className="py-1.5 pr-3">
                    {cell ? <Cell step={cell} host={row.host} /> : <span className="text-muted-foreground">—</span>}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {/* < 640 px: one card per host, its steps in order. */}
      <ul aria-label="Converge" className="flex flex-col gap-2 sm:hidden">
        {view.hosts.map((h) => (
          <li key={h.host_id} data-converge-host={h.host} data-host-state={h.state} className="border border-border p-2">
            <p className="flex items-baseline justify-between gap-2 text-sm font-semibold">
              {h.host}
              <span className="text-xs font-normal text-muted-foreground first-letter:uppercase">{h.state}</span>
            </p>
            <ol aria-label={`Steps on ${h.host}`} className="mt-1 flex flex-col gap-1 text-xs">
              {h.steps.map((s) => (
                <li key={s.position}>
                  <Cell step={s} host={h.host} />
                </li>
              ))}
            </ol>
          </li>
        ))}
      </ul>
    </section>
  )
}

function Actions({ run, view }: { run: RunInfo; view: ConvergeView }) {
  const me = useSession((s) => s.me)
  const cancelRun = useConverge((s) => s.cancelRun)
  const retry = useConverge((s) => s.retry)
  const [replan, setReplan] = useState(false)
  const replanId = useId()
  if (!canOperate(me, run.workspace)) return null
  const done = finished(view)
  const failed = retryableSteps(view)
  return (
    <div className="flex flex-wrap items-center gap-3">
      {!done && active(run.state) && (
        <Button
          size="sm"
          variant="destructive"
          onClick={() => {
            cancelRun(run.id)
          }}
        >
          <StopIcon aria-hidden />
          Cancel converge
        </Button>
      )}
      {done && failed > 0 && (
        <>
          <Button
            size="sm"
            onClick={() => {
              retry(run.id, replan)
            }}
          >
            <ArrowClockwiseIcon aria-hidden />
            Retry failed
          </Button>
          <span className="flex items-center gap-2 text-xs">
            <Checkbox
              id={replanId}
              checked={replan}
              onCheckedChange={(c) => {
                setReplan(c === true)
              }}
            />
            <label htmlFor={replanId}>Re-plan at latest commit</label>
          </span>
        </>
      )}
    </div>
  )
}

export default function ConvergePage({ run }: { run: RunInfo }) {
  const watch = useConverge((s) => s.watch)
  const view = useConverge((s) => s.views[run.id] ?? null)
  const error = useConverge((s) => s.error)

  useEffect(() => {
    watch(run.id)
    return () => {
      watch(null)
    }
  }, [run.id, watch])

  return (
    <Page name="converge" labelledBy="run-title">
      <Link to="/runs" className={`${LINK} flex w-fit items-center gap-1 text-xs text-muted-foreground`}>
        <ArrowLeftIcon aria-hidden className="size-3.5" />
        All runs
      </Link>
      <ErrorLine />
      <PageHeader
        id="run-title"
        title="Converge"
        description={
          <span className="flex flex-wrap items-center gap-2">
            <span data-run-state={run.state}>
              <StateBadge state={run.state} />
            </span>
            <CheckBadge run={run} />
            <span className="font-mono">{short(run.commit)}</span>
            <span>inventory {run.inventory}</span>
          </span>
        }
      />
      {view?.retry_of && (
        <p data-retry-of className="text-xs text-muted-foreground">
          Retries{' '}
          <Link to={`/runs/${view.retry_of}`} className={LINK}>
            converge {short(view.retry_of)}
          </Link>
          .
        </p>
      )}
      <RunChecks run={run} />
      <ApprovalPanel run={run} />
      {error && (
        <p role="alert" className="text-xs text-status-err">
          {error}
        </p>
      )}
      {view ? (
        <>
          <Actions run={run} view={view} />
          <Matrix view={view} />
        </>
      ) : (
        <LoadState loading label="Loading the converge…" />
      )}
      <ConvergePreview />
      <DecisionDialog />
    </Page>
  )
}
