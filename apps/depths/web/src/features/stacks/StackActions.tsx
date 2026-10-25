/**
 * Lifecycle buttons (US2): each asks the server for a preview; nothing runs from here. A stack
 * that cannot be acted on (invalid, unreadable) shows none. The server authorises every one again.
 */
import { ArrowClockwiseIcon, ArrowDownIcon, CloudArrowDownIcon, PlayIcon, StopIcon, UploadIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import type { ReactNode } from 'react'

import { RESTRICTED_OPS, useUntrusted } from '@/features/network/model'
import type { Stack, StackOp } from '@/protocol/generated'

import { blockedReason } from './model'
import { useOps } from './ops-store'

const STACK_OPS: { op: StackOp; label: string; icon: ReactNode }[] = [
  { op: { kind: 'up' }, label: 'Up', icon: <PlayIcon /> },
  { op: { kind: 'stop' }, label: 'Stop', icon: <StopIcon /> },
  { op: { kind: 'restart' }, label: 'Restart', icon: <ArrowClockwiseIcon /> },
  { op: { kind: 'down', options: { remove_volumes: false, remove_images: false } }, label: 'Down', icon: <ArrowDownIcon /> },
  { op: { kind: 'pull' }, label: 'Pull', icon: <CloudArrowDownIcon /> },
  { op: { kind: 'update' }, label: 'Update', icon: <UploadIcon /> },
]

export function StackActions({ stack }: { stack: Stack }) {
  const requestPreview = useOps((s) => s.requestPreview)
  const untrusted = useUntrusted()
  if (blockedReason(stack) !== null) return null
  return (
    <div role="toolbar" aria-label="Stack actions" className="flex min-w-0 flex-wrap gap-1">
      {STACK_OPS.map(({ op, label, icon }) => {
        const blocked = RESTRICTED_OPS.has(op.kind) ? null : untrusted
        return (
          <Button
            key={op.kind}
            variant="outline"
            size="sm"
            data-action={op.kind}
            disabled={blocked !== null}
            title={blocked ?? undefined}
            data-blocked={blocked === null ? undefined : 'untrusted_network'}
            onClick={() => requestPreview({ kind: 'op', stack: stack.name, op })}
          >
            {icon}
            {label}
          </Button>
        )
      })}
    </div>
  )
}

export function ServiceActions({ stack, service }: { stack: Stack; service: string }) {
  const requestPreview = useOps((s) => s.requestPreview)
  if (blockedReason(stack) !== null) return null
  const ask = (op: StackOp): void => requestPreview({ kind: 'op', stack: stack.name, op })
  return (
    <div role="toolbar" aria-label={`Service ${service} actions`} className="flex shrink-0 gap-0.5">
      <Button
        variant="ghost"
        size="icon-xs"
        data-action="service_up"
        aria-label={`Start ${service}`}
        onClick={() => ask({ kind: 'service_up', options: { service } })}
      >
        <PlayIcon />
      </Button>
      <Button
        variant="ghost"
        size="icon-xs"
        data-action="service_stop"
        aria-label={`Stop ${service}`}
        onClick={() => ask({ kind: 'service_stop', options: { service } })}
      >
        <StopIcon />
      </Button>
      <Button
        variant="ghost"
        size="icon-xs"
        data-action="service_restart"
        aria-label={`Restart ${service}`}
        onClick={() => ask({ kind: 'service_restart', options: { service } })}
      >
        <ArrowClockwiseIcon />
      </Button>
    </div>
  )
}
