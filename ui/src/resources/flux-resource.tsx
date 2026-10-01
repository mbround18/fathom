/**
 * Flux GitOps resource (spec 005): reached only via a Helm release's origin-link click-through
 * (`related.tsx`'s `RelatedResourceLink` / `GitOpsOriginRef`), never listed in the navigator —
 * `nav.ts` intentionally has no entry for `section_keys::flux_resource()`. Reuses the backend's
 * existing Flux discovery/summarize logic (`SelectFluxResource`, no new poll loop).
 */
import { StatusBadge } from '@/components/StatusBadge'
import type { FluxResourceSummary } from '@/protocol/types'
import { FLUX_RESOURCE_SECTION } from '@/store/keys'

import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = FLUX_RESOURCE_SECTION

const fluxResourceSections: DetailSection<FluxResourceSummary, FluxResourceSummary>[] = [
  {
    id: 'details',
    title: 'Details',
    render: ({ row, detail }) => {
      const d = detail ?? row
      if (!d) return null
      return {
        type: 'fields',
        fields: [
          { label: 'Kind', value: d.kind, mono: true },
          { label: 'Name', value: `${d.namespace}/${d.name}`, mono: true },
          { label: 'Ready', value: <StatusBadge status={d.ready} /> },
          { label: 'Message', value: d.message || '—' },
          { label: 'Revision', value: d.revision || '—', mono: true },
          { label: 'Source', value: d.source_ref ?? '—', mono: true },
          { label: 'Suspended', value: d.suspended ? 'Yes' : 'No' },
          { label: 'Interval', value: d.interval || '—', mono: true },
          { label: 'Last transition', value: d.last_transition || '—', mono: true },
        ],
      }
    },
  },
]

export const fluxResource: ResourceDescriptor<FluxResourceSummary, FluxResourceSummary> = {
  section: SECTION,
  kind: 'FluxResource',
  label: 'Flux Resource',
  namespaced: true,
  columns: [],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: (r) => ({ section: SECTION, kind: r.kind, namespace: r.namespace, name: r.name }),
  // No list command exists for this kind (US3: a deleted origin resolves to
  // `FluxResourceNotFound`, not a list refresh) — re-issuing the last selection is the closest
  // meaningful "refresh", but `refresh(route)` has no object identity to re-select, so this is
  // effectively unreachable: the section has no navigator entry and no `ResourceTable` refresh
  // button ever targets it.
  refresh: () => ({ type: 'RefreshFlux' }),
  read: (ref) => [
    {
      type: 'SelectFluxResource',
      data: { kind: ref.kind, namespace: ref.namespace ?? '', name: ref.name },
    },
  ],
  detailSections: fluxResourceSections,
  actions: [],
  selectListEvent: () => undefined,
  selectDetail: (d) =>
    d.eventType === 'FluxResourceDetailsUpdated' ? (d.data as FluxResourceSummary) : undefined,
}
