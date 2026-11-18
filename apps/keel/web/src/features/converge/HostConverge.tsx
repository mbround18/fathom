/**
 * Converge on the Hosts page (spec 068 contracts/ui.md "Hosts", "Host page", T028): the Converge
 * column, the bulk "Converge…" action on a selection, and in a host's panel a Converge button and
 * its last converge's steps. Each opens the converge preview (./ConvergePreview.tsx), which the
 * page mounts once.
 */
import { ArrowsClockwiseIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { H3, Link, LINK } from '@companion/ui/kit'
import { useEffect } from 'react'

import { useSession } from '@/app/session-store'
import type { HostInfo } from '@/protocol/wire'

import { canConverge, convergeBlocked, hostConvergeText, stepLabel } from './derive'
import { StepIcon } from './StepIcon'
import { useConverge } from './store'

const TONE = {
  waiting: 'text-muted-foreground',
  holding: 'text-accent-neon',
  succeeded: 'text-status-ok',
  failed: 'text-status-err',
  cancelled: 'text-muted-foreground',
} as const

/** The Converge column: a dash when the host has no converge. */
export function ConvergeCell({ host }: { host: HostInfo }) {
  const c = host.converge
  const steps = useConverge((s) => (c ? s.views[c.run_id]?.hosts.find((h) => h.host_id === host.id)?.steps : undefined))
  if (!c) return <span className="text-muted-foreground">—</span>
  return (
    <Link to={`/runs/${c.run_id}`} data-host-converge={c.state} className={`${LINK} text-xs ${TONE[c.state]}`}>
      {hostConvergeText(c, steps)}
    </Link>
  )
}

/** "Converge…" on the selected hosts; off, with the reason, when the caller can't. */
export function ConvergeSelected({ hosts, binding }: { hosts: readonly HostInfo[]; binding: string | null }) {
  const me = useSession((s) => s.me)
  const disabled = useConverge((s) => s.disabled)
  const preview = useConverge((s) => s.preview)
  if (disabled) return null
  const blocked = convergeBlocked(me, hosts, binding)
  return (
    <span className="inline-flex flex-wrap items-center gap-2">
      <Button
        size="sm"
        variant="outline"
        data-converge-selected
        disabled={blocked !== null}
        aria-describedby={blocked !== null && hosts.length > 0 ? 'converge-blocked' : undefined}
        onClick={() => {
          if (binding === null) return
          preview({ binding_id: binding, hosts: { kind: 'hosts', ids: hosts.map((h) => h.id) }, check: false })
        }}
      >
        <ArrowsClockwiseIcon aria-hidden />
        Converge…
      </Button>
      {blocked !== null && hosts.length > 0 && (
        <span id="converge-blocked" className="text-xs text-muted-foreground">
          {blocked}
        </span>
      )}
    </span>
  )
}

/** In the host's panel: Converge and the last converge's steps. */
export function HostConverge({ host, binding }: { host: HostInfo; binding: string | null }) {
  const me = useSession((s) => s.me)
  const disabled = useConverge((s) => s.disabled)
  const preview = useConverge((s) => s.preview)
  const fetch = useConverge((s) => s.fetch)
  const run = host.converge?.run_id ?? null
  const steps = useConverge((s) => (run ? s.views[run]?.hosts.find((h) => h.host_id === host.id)?.steps : undefined))
  useEffect(() => {
    if (run !== null) fetch(run)
  }, [run, fetch])
  if (disabled) return null
  const may = binding !== null && host.state !== 'revoked' && canConverge(me, host)

  return (
    <section aria-label="Converge" className="flex min-w-0 flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-2">
        <h3 className={H3}>Last converge</h3>
        {may && (
          <Button
            size="xs"
            variant="outline"
            className="ml-auto"
            data-converge-host
            onClick={() => {
              preview({ binding_id: binding, hosts: { kind: 'hosts', ids: [host.id] }, check: false })
            }}
          >
            <ArrowsClockwiseIcon aria-hidden />
            Converge
          </Button>
        )}
      </div>
      {host.converge === null ? (
        <p className="text-muted-foreground">Not converged yet.</p>
      ) : (
        <>
          <p>
            <Link to={`/runs/${host.converge.run_id}`} className={`${LINK} ${TONE[host.converge.state]}`}>
              {hostConvergeText(host.converge, steps)}
            </Link>
          </p>
          {steps && steps.length > 0 && (
            <ol aria-label="Last converge steps" className="flex flex-col gap-0.5">
              {steps.map((s) => (
                <li key={s.position} data-step-state={s.state} className="flex items-center gap-1.5">
                  <StepIcon state={s.state} />
                  <span className="font-mono">{s.job}</span>
                  <span className="text-muted-foreground">{stepLabel(s.state)}</span>
                </li>
              ))}
            </ol>
          )}
        </>
      )}
    </section>
  )
}
