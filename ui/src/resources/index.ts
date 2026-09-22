/**
 * The descriptor registry, keyed by `SectionKey` (T060). T068–T075 register the remaining kinds.
 */
import type { SectionKey } from '@/protocol/types'

import { applications } from './applications'
import { deployments } from './deployments'
import { nodes } from './nodes'
import { pods } from './pods'
import type { AnyDescriptor } from './types'

export type * from './types'

export const ALL_DESCRIPTORS: readonly AnyDescriptor[] = [pods, deployments, nodes, applications]

const BY_SECTION = new Map<SectionKey, AnyDescriptor>(ALL_DESCRIPTORS.map((d) => [d.section, d]))

export function descriptorFor(section: SectionKey | undefined): AnyDescriptor | undefined {
  return section === undefined ? undefined : BY_SECTION.get(section)
}

/** Stable identity of a row under its descriptor (for selection and the inspector). */
export function rowKeyOf(d: AnyDescriptor, row: unknown): string {
  return d.rowKey(row as never)
}
