/**
 * The descriptor registry, keyed by `SectionKey` (T060). T068–T075 register the remaining kinds.
 */
import type { SectionKey } from '@/protocol/types'

import { applications } from './applications'
import { daemonsets } from './daemonsets'
import { deployments } from './deployments'
import { fluxResource } from './flux-resource'
import { helm } from './helm'
import { nodes } from './nodes'
import { pods } from './pods'
import { statefulsets } from './statefulsets'
import type { AnyDescriptor } from './types'

export type * from './types'

export const ALL_DESCRIPTORS: readonly AnyDescriptor[] = [
  pods,
  deployments,
  statefulsets,
  daemonsets,
  nodes,
  applications,
  helm,
]

/**
 * Detail-only descriptors (spec 005): reachable exclusively via `openInspector`'s
 * origin-link click-through, never through a `ResourceTable`/nav entry, and with no backend
 * list command at all — so they sit outside `ALL_DESCRIPTORS` (and `descriptors.test.ts`'s
 * uniform "every kind is listable" contract) but still resolve through `descriptorFor`.
 * `FluxResourceSummary` is also never a `ListRow`, so the cast mirrors the bivariant-registry
 * trick `types.ts` documents for `ALL_DESCRIPTORS`, needed explicitly here since it's a type
 * outside the `ResourceListEvent` union entirely.
 */
const DETAIL_ONLY_DESCRIPTORS: readonly AnyDescriptor[] = [fluxResource as unknown as AnyDescriptor]

const BY_SECTION = new Map<SectionKey, AnyDescriptor>(
  [...ALL_DESCRIPTORS, ...DETAIL_ONLY_DESCRIPTORS].map((d) => [d.section, d]),
)

export function descriptorFor(section: SectionKey | undefined): AnyDescriptor | undefined {
  return section === undefined ? undefined : BY_SECTION.get(section)
}

/** Stable identity of a row under its descriptor (for selection and the inspector). */
export function rowKeyOf(d: AnyDescriptor, row: unknown): string {
  return d.rowKey(row as never)
}
