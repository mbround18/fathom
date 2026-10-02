/** Non-component helpers shared by the action menus and dialogs (US3). */
import { useMemo } from 'react'

import { capabilityForSection, type CapabilityTarget } from '@/lib/capabilities'
import type { ActionDescriptor, AnyDescriptor } from '@/resources'
import { useStore, type ListRow, type PaneId } from '@/store'

export interface ConfirmItem {
  namespace: string | null
  name: string
}

/**
 * More than this many objects requires typing the count. At 1, only a single-object delete — whose
 * manifest is one line the user can read in full — arms on first paint; every bulk action makes
 * them state how many they meant.
 */
export const TYPED_CONFIRM_THRESHOLD = 1

export function itemName(i: ConfirmItem): string {
  return i.namespace ? `${i.namespace}/${i.name}` : i.name
}

/** The largest count the dialog accepts (a typo guard, not a cluster limit). */
export const MAX_REPLICAS = 1000

export function parseReplicas(text: string): number | undefined {
  const t = text.trim()
  if (!/^\d+$/.test(t)) return undefined
  const n = Number(t)
  return Number.isSafeInteger(n) && n <= MAX_REPLICAS ? n : undefined
}

/** The irreversibility warning shown by the debug-container dialog (025, FR-013). */
export const EPHEMERAL_WARNING =
  'Ephemeral containers cannot be removed once added and will persist for the life of the pod.'

/** Enabled once an image is entered and a target container is resolved. */
export function debugInputValid(image: string, target: string | undefined): boolean {
  return image.trim().length > 0 && (target?.trim().length ?? 0) > 0
}

const NO_KEYS: readonly string[] = []

/** Rows of `rows` whose key is in the pane's multi-select, in list order. */
export function useCheckedRows(
  paneId: PaneId,
  descriptor: AnyDescriptor | undefined,
  rows: readonly ListRow[],
): ListRow[] {
  const checked = useStore((s) => s.selection[paneId] ?? NO_KEYS)
  return useMemo(() => {
    if (!descriptor || checked.length === 0) return []
    const set = new Set(checked)
    return rows.filter((r) => set.has(descriptor.rowKey(r)))
  }, [checked, descriptor, rows])
}

/** Open the dialog for `action` on `rows`. */
export function promptAction(
  descriptor: AnyDescriptor,
  action: ActionDescriptor<ListRow>,
  rows: readonly ListRow[],
  opts: { paneId?: PaneId; returnFocus?: HTMLElement | null } = {},
): void {
  if (rows.length === 0) return
  useStore.getState().openAction({
    section: descriptor.section,
    actionId: action.id,
    rows: [...rows],
    ...(opts.paneId !== undefined ? { paneId: opts.paneId } : {}),
    returnFocus: opts.returnFocus ?? null,
  })
}

const ACTION_VERB: Record<
  ActionDescriptor<unknown>['id'],
  Pick<CapabilityTarget, 'verb' | 'subresource'> &
    Partial<Pick<CapabilityTarget, 'resource' | 'apiGroup'>>
> = {
  delete: { verb: 'delete' },
  scale: { verb: 'update', subresource: 'scale' },
  restart: { verb: 'patch' },
  debug: { verb: 'patch', subresource: 'ephemeralcontainers' },
  suspend: { verb: 'patch' },
  resume: { verb: 'patch' },
  // Triggering a CronJob creates a Job.
  trigger: { verb: 'create', resource: 'jobs', apiGroup: 'batch' },
}

/**
 * The capability an action needs on `rows`: one namespace when they all share it, else cluster
 * scope (the stricter question), so a mixed selection never passes on one namespace's grant.
 */
export function actionTarget(
  descriptor: AnyDescriptor,
  action: ActionDescriptor<ListRow>,
  rows: readonly ListRow[],
): CapabilityTarget {
  const namespaces = new Set(rows.map((r) => descriptor.ref(r).namespace ?? null))
  const [only] = [...namespaces]
  return {
    ...capabilityForSection(descriptor.section),
    ...ACTION_VERB[action.id],
    namespace: namespaces.size === 1 && only !== undefined ? only : null,
  }
}
