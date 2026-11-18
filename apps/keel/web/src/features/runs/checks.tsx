/**
 * Spec 068 pieces every run surface shares: the `check` badge (FR-011) and the required forge
 * checks a run waits on (US7, contracts/ui.md "`waiting_checks` state shows the contexts with
 * their states and forge links"). Each state is an icon and a word, never colour alone.
 */
import { ArrowSquareOutIcon, CheckCircleIcon, CircleNotchIcon, WarningOctagonIcon } from '@phosphor-icons/react'
import { Badge } from '@companion/ui'
import { LINK } from '@companion/ui/kit'

import type { RunCheckView, RunInfo } from '@/protocol/wire'

import { checkLabel, checksSummary, checkTone, forgeLink } from './run-068'

/** A check run (`ansible-playbook --check --diff`): it changes nothing. */
export function CheckBadge({ run }: { run: Pick<RunInfo, 'check'> }) {
  if (!run.check) return null
  return (
    <Badge variant="outline" data-check-badge title="Check mode: ansible-playbook --check --diff, changes nothing">
      check
    </Badge>
  )
}

const ICON = {
  ok: { Icon: CheckCircleIcon, className: 'text-status-ok' },
  err: { Icon: WarningOctagonIcon, className: 'text-status-err' },
  wait: { Icon: CircleNotchIcon, className: 'text-muted-foreground' },
} as const

function Check({ check }: { check: RunCheckView }) {
  const tone = checkTone(check.state)
  const { Icon, className } = ICON[tone]
  const href = forgeLink(check.target_url)
  return (
    <li data-check={check.context} data-check-state={check.state} className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-2">
      <Icon aria-hidden weight="bold" className={`size-3.5 mt-0.5 shrink-0 self-start ${className}`} />
      <span className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
        <span className="min-w-0 font-mono break-all">{check.context}</span>
        <span className={tone === 'err' ? 'font-medium text-status-err' : 'text-muted-foreground'}>{checkLabel(check.state)}</span>
        {href && (
          <a href={href} target="_blank" rel="noreferrer noopener" className={`${LINK} inline-flex items-center gap-0.5`}>
            details
            <span className="sr-only"> for {check.context} on the forge</span>
            <ArrowSquareOutIcon aria-hidden className="size-3" />
          </a>
        )}
      </span>
    </li>
  )
}

/**
 * The run's required checks, while it waits on them or once they decided it. Nothing when its
 * binding required none when it was made.
 */
export function RunChecks({ run }: { run: Pick<RunInfo, 'checks' | 'state'> }) {
  const checks = run.checks ?? []
  if (checks.length === 0) return null
  const waiting = run.state === 'waiting_checks'
  return (
    <section
      aria-label="Required checks"
      data-run-checks={waiting ? 'waiting' : 'settled'}
      className={`flex min-w-0 flex-col gap-1 border px-3 py-2 text-xs ${waiting ? 'border-status-warn/50' : 'border-border'}`}
    >
      <p className="flex flex-wrap gap-x-2">
        <span className="font-medium">{waiting ? 'Waiting for the forge’s checks' : 'Required checks'}</span>
        <span className="text-muted-foreground">{checksSummary(checks)}</span>
      </p>
      {waiting && (
        <p className="text-muted-foreground">Nothing is sent to a host until every check below passes on this commit.</p>
      )}
      <ul className="flex min-w-0 flex-col gap-1">
        {checks.map((c) => (
          <Check key={c.context} check={c} />
        ))}
      </ul>
    </section>
  )
}
