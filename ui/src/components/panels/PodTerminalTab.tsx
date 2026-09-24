/**
 * Terminal tab (Phase 4, US2): an interactive `kube exec` shell rendered with xterm.js,
 * driven by `StartPodExec`/`SendPodExecInput`/`ResizePodExec`/`StopPodExec` and the
 * `PodExecReady`/`PodExecOutputChunk`/`StreamClosed` events (contracts/protocol.md).
 *
 * The terminal surface and stream tracking live in the shared `TerminalView`
 * (037-node-shell T002); this tab adds the container picker and the ended/error banner.
 */
import { PlugsIcon, ShieldWarningIcon, WarningIcon } from '@phosphor-icons/react'
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import type { PodDetails } from '@/protocol/types'
import type { ObjectRef } from '@/resources'
import { detailKey, useStore } from '@/store'

import { Picker } from '../Picker'
import { Loading } from '../shell/PaneStates'
import { TerminalView, type TerminalPhase } from './TerminalView'

export function PodTerminalTab({ target }: { target: ObjectRef }) {
  const namespace = target.namespace ?? ''
  const name = target.name
  const context = useStore((s) => s.session.activeContext)
  const detailState = useStore((s) => s.details[detailKey(context, 'Pod', namespace, name)])
  const detail =
    detailState?.eventType === 'PodDetailsUpdated' ? (detailState.data as PodDetails) : undefined
  const containers = detail?.containers ?? []

  // FR-008: with more than one container, the operator picks one before the session starts.
  const [picked, setPicked] = useState<string | undefined>(undefined)
  const chosen = containers.length > 1 ? picked : containers[0]
  const started = chosen !== undefined

  const [status, setStatus] = useState<{ phase: TerminalPhase; reason?: string }>({
    phase: 'attaching',
  })
  const phase = started ? status.phase : 'picking'

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {containers.length > 1 && !started && (
        <div className="flex shrink-0 items-center gap-2 border-b border-border px-2 py-1.5">
          <Picker
            label="Container"
            prefix="container"
            value={picked}
            options={containers.map((c) => ({ value: c, label: c }))}
            onChange={setPicked}
          />
          <span className="text-xs text-muted-foreground">
            Choose a container to attach the terminal to.
          </span>
        </div>
      )}
      {phase === 'picking' && containers.length === 0 && <Loading rows={6} label="Loading pod" />}
      {started && (
        <TerminalView
          key={`${namespace}/${name}/${chosen}`}
          namespace={namespace}
          name={name}
          container={containers.length > 1 ? chosen : null}
          onPhase={(p, reason) => setStatus({ phase: p, reason })}
        />
      )}
      {(phase === 'ended' || phase === 'error') && (
        <div
          role="status"
          data-pane-state={phase === 'error' ? 'terminal-error' : 'terminal-ended'}
          className="flex shrink-0 items-center gap-2 border-t border-status-warn/40 bg-status-warn/10 px-3 py-1.5 text-xs text-status-warn"
        >
          {phase === 'error' ? (
            status.reason?.toLowerCase().includes('permission') ? (
              <ShieldWarningIcon aria-hidden="true" className="size-4" />
            ) : (
              <WarningIcon aria-hidden="true" className="size-4" />
            )
          ) : (
            <PlugsIcon aria-hidden="true" className="size-4" />
          )}
          {/* FR-013: the backend's error reason is shown verbatim, not a generic message. */}
          <span>
            {phase === 'error' ? (status.reason ?? 'Failed to attach.') : 'Session ended.'}
          </span>
          {containers.length > 1 && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="ml-auto"
              onClick={() => setPicked(undefined)}
            >
              Choose container
            </Button>
          )}
        </div>
      )}
    </div>
  )
}
