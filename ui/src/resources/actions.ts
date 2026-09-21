/**
 * Builders for the US3 mutating actions (T094). Each descriptor lists the actions its kind
 * supports in protocol v1 (`src/protocol/v1/cmd.rs`); `runAction` (store/actions.ts) sends them.
 */
import type { BackendCmd } from '@/protocol/types'

import type { ActionDescriptor } from './types'

/** Any namespaced list row. */
export interface Named {
  namespace: string
  name: string
}

type Make = (namespace: string, name: string) => BackendCmd

/**
 * Delete, one `Delete<Kind>` per row. `bulk`, when given, replaces them with a single command
 * for two or more rows (pods: `BulkDelete`, which the backend applies to pods only).
 */
export function deleteAction<R extends Named>(
  make: Make,
  bulk?: (rows: R[]) => BackendCmd,
): ActionDescriptor<R> {
  return {
    id: 'delete',
    label: 'Delete',
    done: 'Deleted',
    doing: 'Deleting',
    prompt: 'confirm',
    destructive: true,
    bulk: true,
    plan: (rows) =>
      bulk && rows.length > 1
        ? [{ cmd: bulk(rows), rows }]
        : rows.map((r) => ({ cmd: make(r.namespace, r.name), rows: [r] })),
  }
}

/** Rollout restart, one `Restart<Kind>` per row. */
export function restartAction<R extends Named>(make: Make): ActionDescriptor<R> {
  return {
    id: 'restart',
    label: 'Restart',
    done: 'Restarted',
    doing: 'Restarting',
    prompt: 'confirm',
    bulk: true,
    plan: (rows) => rows.map((r) => ({ cmd: make(r.namespace, r.name), rows: [r] })),
  }
}

/** Scale one object to `input.replicas` (≥ 0). */
export function scaleAction<R extends Named>(
  make: (namespace: string, name: string, replicas: number) => BackendCmd,
  current: (row: R) => number,
): ActionDescriptor<R> {
  return {
    id: 'scale',
    label: 'Scale',
    done: 'Scaled',
    doing: 'Scaling',
    prompt: 'scale',
    current,
    plan: (rows, input) => {
      const replicas = input.replicas
      if (replicas === undefined || !Number.isInteger(replicas) || replicas < 0) return []
      return rows.map((r) => ({ cmd: make(r.namespace, r.name, replicas), rows: [r] }))
    },
  }
}

/** The actions that can run on `count` rows at once. */
export function actionsFor<R>(
  actions: ActionDescriptor<R>[],
  count: number,
): ActionDescriptor<R>[] {
  if (count === 0) return []
  return count === 1 ? actions : actions.filter((a) => a.bulk)
}
