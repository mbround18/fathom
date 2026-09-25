/** VerticalPodAutoscalers (CRD autoscaling.k8s.io/v1): target, update mode, per-container recommendations. */
import type {
  VerticalPodAutoscalerDetails,
  VerticalPodAutoscalerSummary,
  VpaContainerRecommendation,
  VpaResources,
} from '@/protocol/types'

import { deleteAction } from './actions'
import { labelSections, metadataSection } from './configCommon'
import { compareText } from './search'
import type { ResourceDescriptor } from './types'

const SECTION = 'config/verticalpodautoscalers'

/** `cpu 25m / mem 100Mi`, dashes for what the recommender has not reported. */
export function vpaResources(r: VpaResources): string {
  return `cpu ${r.cpu || '—'} / mem ${r.memory || '—'}`
}

/** One line per container: lower / target / upper. */
export function vpaRecommendationLine(r: VpaContainerRecommendation): string {
  return `${r.container}: lower ${vpaResources(r.lower_bound)} · target ${vpaResources(r.target)} · upper ${vpaResources(r.upper_bound)}`
}

/** Compact list-cell text for the first container's target (`+N` when several). */
export function vpaTargetSummary(recs: VpaContainerRecommendation[]): string {
  const first = recs[0]
  if (!first) return '—'
  const t = `${first.target.cpu || '—'} / ${first.target.memory || '—'}`
  return recs.length > 1 ? `${t} +${String(recs.length - 1)}` : t
}

export const verticalpodautoscalers: ResourceDescriptor<
  VerticalPodAutoscalerSummary,
  VerticalPodAutoscalerDetails
> = {
  section: SECTION,
  kind: 'VerticalPodAutoscaler',
  label: 'Vertical Pod Autoscalers',
  namespaced: true,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 3,
      cell: (r) => r.name,
      text: (r) => r.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'mode',
      header: 'Mode',
      priority: 1,
      minWidth: 80,
      cell: (r) => r.update_mode,
      text: (r) => r.update_mode,
      sort: (a, b) => compareText(a.update_mode, b.update_mode),
    },
    {
      id: 'target',
      header: 'Target',
      priority: 2,
      minWidth: 140,
      grow: 2,
      cell: (r) => r.target_ref,
      text: (r) => r.target_ref,
      sort: (a, b) => compareText(a.target_ref, b.target_ref),
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (r) => r.namespace,
      text: (r) => r.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'recommended',
      header: 'Recommended (cpu / mem)',
      priority: 3,
      minWidth: 150,
      grow: 2,
      cell: (r) => vpaTargetSummary(r.recommendations),
      text: (r) => vpaTargetSummary(r.recommendations),
      sort: (a, b) =>
        compareText(vpaTargetSummary(a.recommendations), vpaTargetSummary(b.recommendations)),
    },
  ],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: (r) => ({
    section: SECTION,
    kind: 'VerticalPodAutoscaler',
    namespace: r.namespace,
    name: r.name,
  }),
  refresh: () => ({ type: 'RefreshVerticalPodAutoscalers' }),
  read: (ref) => [
    {
      type: 'SelectVerticalPodAutoscaler',
      data: { namespace: ref.namespace ?? '', name: ref.name },
    },
    { type: 'RefreshEvents' },
  ],
  detailSections: [
    metadataSection(true),
    {
      id: 'scaling',
      title: 'Scaling',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return {
          type: 'fields',
          fields: [
            { label: 'Scale target', value: d.target_ref || '—', mono: true },
            { label: 'Update mode', value: d.update_mode, mono: true },
          ],
        }
      },
    },
    {
      id: 'recommendations',
      title: 'Recommendations',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return d.recommendations.length === 0
          ? { type: 'text', text: 'No recommendation yet.' }
          : {
              type: 'list',
              items: d.recommendations.map((r) => (
                <span key={r.container} className="font-mono" data-testid="vpa-recommendation">
                  {vpaRecommendationLine(r)}
                </span>
              )),
            }
      },
    },
    {
      id: 'conditions',
      title: 'Conditions',
      render: ({ detail }) =>
        detail
          ? detail.conditions.length === 0
            ? { type: 'text', text: 'No conditions reported.' }
            : { type: 'list', items: detail.conditions.map((c) => <span key={c}>{c}</span>) }
          : null,
    },
    ...labelSections<VerticalPodAutoscalerSummary, VerticalPodAutoscalerDetails>(),
  ],
  actions: [
    {
      ...deleteAction((namespace, name) => ({
        type: 'DeleteVerticalPodAutoscaler',
        data: { namespace, name },
      })),
      accessCheck: (row) => ({
        verb: 'delete',
        group: 'autoscaling.k8s.io',
        resource: 'verticalpodautoscalers',
        namespace: row.namespace,
      }),
    },
  ],
  selectListEvent: (e) => (e.type === 'VerticalPodAutoscalersUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'VerticalPodAutoscalerDetailsUpdated'
      ? (d.data as VerticalPodAutoscalerDetails)
      : undefined,
  rawJson: (d) => d.raw_json,
}
