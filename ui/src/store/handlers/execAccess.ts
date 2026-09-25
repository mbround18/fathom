/**
 * `PodExecAccessChecked` (024-pod-exec-terminal): the reply to `CheckPodExecAccess`, stored
 * per `namespace/name/container` with a bumped `rev` so `PodTerminalTab` can tell a fresh
 * result from a stale one.
 */
import type { ResourceReadEvent } from '@/protocol/types'

import type { Draft } from '../draft'
import { execAccessKey } from '../index'

export type ExecAccessEvent = Extract<ResourceReadEvent, { type: 'PodExecAccessChecked' }>

export function handleExecAccess(d: Draft, event: ExecAccessEvent): void {
  const { namespace, name, container, allowed, reason } = event.data
  const key = execAccessKey(namespace, name, container)
  const slice = d.edit('execAccess')
  slice[key] = { allowed, reason: reason ?? undefined, rev: (slice[key]?.rev ?? 0) + 1 }
}
