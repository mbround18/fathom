/**
 * 036 US2: per-context daemon status chip; opens the inspectable detail + preference toggle.
 * 043 US4: the chip always names itself ("Daemon: Off"), and the dialog explains what the
 * daemon is and what its switch does.
 */
import { CopyIcon, HardDrivesIcon } from '@phosphor-icons/react'
import { useState } from 'react'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { cn } from '@/lib/utils'
import { useStore } from '@/store'

import { DaemonSwitch } from './DaemonSwitch'
import {
  CLUSTER_HISTORY_SUBTITLE,
  daemonState,
  INSTALL_COMMAND,
  STATE_GLYPH,
  STATE_HELP,
  STATE_LABEL,
  STATE_TONE,
  useActiveDaemon,
} from './state'

const TONE_CLASS = {
  ok: 'text-status-ok',
  warn: 'text-status-warn',
  err: 'text-status-err',
  muted: 'text-muted-foreground',
} as const

const DESCRIPTION_ID = 'daemon-chip-description'

export function DaemonChip({ compact }: { compact: boolean }) {
  const [open, setOpen] = useState(false)
  const { context, daemon } = useActiveDaemon()
  const refresh = useStore((s) => s.refreshDaemon)
  const navigate = useStore((s) => s.navigate)
  if (!context) return null

  const state = daemonState(daemon)

  const tone = STATE_TONE[state]
  const enabled = state !== 'off'
  const caps = daemon.status?.capabilities
  const detail = daemon.status?.detail

  const copy = () => {
    void navigator.clipboard.writeText(INSTALL_COMMAND).then(() => {
      toast.success('Copied install command')
    })
  }

  return (
    <>
      <span id={DESCRIPTION_ID} className="sr-only">
        {STATE_HELP[state]}
      </span>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogTrigger asChild>
          <Button
            variant="ghost"
            size={compact ? 'icon' : 'sm'}
            aria-label={`Daemon: ${STATE_LABEL[state]}`}
            aria-describedby={DESCRIPTION_ID}
            data-testid="daemon-chip"
            data-daemon-state={state}
            className={cn(
              'min-h-[var(--touch-target)] min-w-[var(--touch-target)] shrink-0',
              TONE_CLASS[tone],
            )}
          >
            <HardDrivesIcon />
            {!compact && (
              <span className="flex items-center gap-1 text-xs">
                Daemon:
                <span aria-hidden="true">{STATE_GLYPH[state]}</span>
                {STATE_LABEL[state]}
              </span>
            )}
          </Button>
        </DialogTrigger>
        <DialogContent
          className="max-h-[85vh] overflow-y-auto sm:max-w-lg"
          data-testid="daemon-dialog"
        >
          <DialogHeader>
            <DialogTitle>In-cluster daemon</DialogTitle>
            <DialogDescription>
              Fathom's optional, read-only helper that can run inside{' '}
              <span className="text-accent-neon">{context}</span>. It records what changes in the
              cluster, even while Fathom is closed, and serves a faster topology. Everything works
              without it, and Fathom never installs it.
            </DialogDescription>
          </DialogHeader>

          <DaemonSwitch />

          <div className="flex flex-col gap-1 text-sm" data-testid="daemon-detail">
            <p className={cn('flex items-center gap-1.5 font-semibold', TONE_CLASS[tone])}>
              <span aria-hidden="true">{STATE_GLYPH[state]}</span>
              <span data-testid="daemon-state-label">{STATE_LABEL[state]}</span>
            </p>
            <p className="text-muted-foreground">{STATE_HELP[state]}</p>
            {detail && (
              <p className="break-words whitespace-pre-wrap" data-testid="daemon-error">
                {detail}
              </p>
            )}
            {daemon.status?.nextRetryAt && (
              <p className="text-xs text-muted-foreground">
                Next retry {new Date(daemon.status.nextRetryAt).toLocaleTimeString()}
              </p>
            )}
            {caps && (
              <dl
                className="mt-1 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs"
                data-testid="daemon-capabilities"
              >
                <dt className="text-muted-foreground">Version</dt>
                <dd>{caps.daemonVersion}</dd>
                <dt className="text-muted-foreground">Protocol</dt>
                <dd>
                  {caps.protocolMajor}.{caps.protocolMinor}
                </dd>
                <dt className="text-muted-foreground">Scope</dt>
                <dd className="break-words">
                  {caps.scope.mode === 'cluster' ? 'cluster' : caps.scope.namespaces.join(', ')}
                </dd>
                <dt className="text-muted-foreground">Retention</dt>
                <dd>
                  {caps.retention.maxEvents} events / {Math.round(caps.retention.maxAgeSecs / 3600)}{' '}
                  h
                </dd>
              </dl>
            )}
          </div>

          {(state === 'off' || state === 'notInstalled') && (
            <div className="flex flex-col gap-1" data-testid="daemon-install-hint">
              <p className="text-xs text-muted-foreground">
                To install it yourself, run this against the cluster (Fathom never does this for
                you):
              </p>
              <div className="flex items-center gap-2">
                <code className="min-w-0 flex-1 overflow-x-auto border border-border bg-muted px-2 py-1 text-xs">
                  {INSTALL_COMMAND}
                </code>
                <Button
                  variant="outline"
                  size="icon"
                  aria-label="Copy install command"
                  data-testid="daemon-copy-install"
                  onClick={copy}
                  className="size-[var(--touch-target)] shrink-0"
                >
                  <CopyIcon />
                </Button>
              </div>
            </div>
          )}

          <div className="flex flex-wrap gap-2">
            {enabled && (
              <Button
                variant="outline"
                data-testid="daemon-refresh"
                onClick={() => {
                  refresh(context)
                }}
                className="min-h-[var(--touch-target)]"
              >
                Re-check now
              </Button>
            )}
            {state === 'available' && (
              <Button
                variant="outline"
                data-testid="daemon-open-timeline"
                onClick={() => {
                  navigate('cluster-history')
                  setOpen(false)
                }}
                className="min-h-[var(--touch-target)]"
              >
                Open Cluster history
              </Button>
            )}
          </div>
          {state === 'available' && (
            <p className="text-xs text-muted-foreground">{CLUSTER_HISTORY_SUBTITLE}</p>
          )}
        </DialogContent>
      </Dialog>
    </>
  )
}
