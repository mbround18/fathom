/** 036 US2: per-context daemon status chip; opens the inspectable detail + preference toggle. */
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
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'
import { useStore } from '@/store'

import {
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

export function DaemonChip({ compact }: { compact: boolean }) {
  const [open, setOpen] = useState(false)
  const { context, daemon } = useActiveDaemon()
  const setPreference = useStore((s) => s.setDaemonPreference)
  const refresh = useStore((s) => s.refreshDaemon)
  const openRoot = useStore((s) => s.openRoot)
  const hasDock = useStore((s) => s.layout.profile.dock !== undefined)
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
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button
          variant="ghost"
          size={compact ? 'icon' : 'sm'}
          aria-label={`In-cluster daemon: ${STATE_LABEL[state]}`}
          data-testid="daemon-chip"
          data-daemon-state={state}
          className={cn('shrink-0', TONE_CLASS[tone])}
        >
          <HardDrivesIcon />
          {!compact && (
            <span className="flex items-center gap-1 text-xs">
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
            Optional, read-only helper for <span className="text-accent-neon">{context}</span>.
          </DialogDescription>
        </DialogHeader>

        <div className="flex min-h-[var(--touch-target)] items-center justify-between gap-3 text-sm">
          <span id="daemon-toggle-label">Use in-cluster daemon if present</span>
          <Switch
            checked={enabled}
            onCheckedChange={(on) => {
              setPreference(context, on ? 'auto' : 'off')
            }}
            data-testid="daemon-toggle"
            aria-labelledby="daemon-toggle-label"
          />
        </div>

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
                {caps.retention.maxEvents} events / {Math.round(caps.retention.maxAgeSecs / 3600)} h
              </dd>
            </dl>
          )}
        </div>

        {(state === 'off' || state === 'notInstalled') && (
          <div className="flex flex-col gap-1" data-testid="daemon-install-hint">
            <p className="text-xs text-muted-foreground">
              To install it yourself, run this against the cluster (Fathom never does this for you):
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
          {state === 'available' && !hasDock && (
            <Button
              variant="outline"
              data-testid="daemon-open-timeline"
              onClick={() => {
                openRoot('tool', 'daemon')
                setOpen(false)
              }}
              className="min-h-[var(--touch-target)]"
            >
              Open timeline
            </Button>
          )}
        </div>
        {state === 'available' && hasDock && (
          <p className="text-xs text-muted-foreground">
            Events are in the “While you were away” dock tab.
          </p>
        )}
      </DialogContent>
    </Dialog>
  )
}
