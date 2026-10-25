/**
 * The buttons that open a tab (logs, a shell, stats) from the stack views. Light: no stream or
 * terminal code loads until a tab exists.
 */
import { ChartBarIcon, ScrollIcon, TerminalWindowIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'

import { useSession } from '@/app/session-store'
import { useUntrusted } from '@/features/network/model'
import { shellGate } from '@/features/stacks/security'
import type { ContainerSecurity, LogTarget, StatsTarget } from '@/protocol/generated'

import { useTabs } from './store'

export function OpenLogs({ target, label }: { target: LogTarget; label: string }) {
  const openTab = useTabs((s) => s.openTab)
  return (
    <Button variant="ghost" size="icon-xs" data-open="logs" aria-label={`Logs for ${label}`} title="Logs" onClick={() => openTab({ kind: 'logs', target })}>
      <ScrollIcon aria-hidden />
    </Button>
  )
}

export function OpenStats({ target, label }: { target: StatsTarget; label: string }) {
  const openTab = useTabs((s) => s.openTab)
  return (
    <Button variant="ghost" size="icon-xs" data-open="stats" aria-label={`Stats for ${label}`} title="Stats" onClick={() => openTab({ kind: 'stats', target })}>
      <ChartBarIcon aria-hidden />
    </Button>
  )
}

/**
 * Hidden while the shell feature is off (FR-027): the server refuses it anyway. Disabled, saying
 * why, when the container's rating keeps its shell from this person (051 FR-005); it stays
 * focusable so the reason can be read. The server re-checks the container at open either way.
 */
export function OpenShell({ container, security }: { container: string; security?: ContainerSecurity }) {
  const shellOn = useSession((s) => s.me?.features.shell ?? false)
  const role = useSession((s) => s.me?.role)
  const openTab = useTabs((s) => s.openTab)
  const blocked = useUntrusted()
  if (!shellOn) return null
  const gate = shellGate(security, role)
  return (
    <Button
      variant="ghost"
      size="icon-xs"
      data-open="shell"
      disabled={blocked !== null}
      data-blocked={blocked === null ? undefined : 'untrusted_network'}
      aria-disabled={!gate.enabled || undefined}
      data-disabled-reason={gate.reason ?? undefined}
      aria-label={gate.reason === null ? `Shell in ${container}` : `Shell in ${container}: ${gate.reason}`}
      title={blocked ?? gate.reason ?? 'Shell'}
      className={gate.enabled ? undefined : 'cursor-not-allowed opacity-50'}
      onClick={() => gate.enabled && openTab({ kind: 'shell', container })}
    >
      <TerminalWindowIcon aria-hidden />
    </Button>
  )
}
