/** HorizontalPodAutoscalers (batch D): replicas min/max/current/desired, metric targets, conditions. */
import type {
  HorizontalPodAutoscalerDetails,
  HorizontalPodAutoscalerSummary,
} from '@/protocol/types'

import { deleteAction } from './actions'
import { arr, labelSections, metadataSection, obj, parseJson, str } from './configCommon'
import { compareNumber, compareText } from './search'
import type { ResourceDescriptor } from './types'

const SECTION = 'config/horizontalpodautoscalers'
const n = (v: number | null | undefined) => (v === null || v === undefined ? '—' : String(v))

const scalar = (v: unknown): string =>
  typeof v === 'string' || typeof v === 'number' ? String(v) : ''

function metricValue(t: Record<string, unknown>): string {
  if (t.averageUtilization !== undefined) return `${scalar(t.averageUtilization)}%`
  if (t.averageValue !== undefined) return `avg ${scalar(t.averageValue)}`
  if (t.value !== undefined) return scalar(t.value)
  return '—'
}

export interface HpaMetric {
  label: string
  target: string
  current: string
}

/** Metric targets (spec.metrics) paired with current values (status.currentMetrics) by index. */
export function hpaMetrics(raw: string): HpaMetric[] {
  const j = parseJson(raw)
  const specs = arr(obj(j.spec).metrics).map(obj)
  const cur = arr(obj(j.status).currentMetrics).map(obj)
  return specs.map((m, i) => {
    const kind = str(m.type)
    const key = kind.charAt(0).toLowerCase() + kind.slice(1)
    const inner = obj(m[key])
    const name = str(obj(inner.metric).name) || str(inner.name) || kind
    const current = cur[i] ? obj(obj(cur[i][key]).current) : {}
    return {
      label: `${kind}: ${name}`,
      target: metricValue(obj(inner.target)),
      current: metricValue(current),
    }
  })
}

export const horizontalpodautoscalers: ResourceDescriptor<
  HorizontalPodAutoscalerSummary,
  HorizontalPodAutoscalerDetails
> = {
  section: SECTION,
  kind: 'HorizontalPodAutoscaler',
  label: 'Horizontal Pod Autoscalers',
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
      id: 'replicas',
      header: 'Replicas',
      priority: 1,
      minWidth: 80,
      align: 'end',
      cell: (r) => `${n(r.current_replicas)}/${String(r.max_replicas)}`,
      text: (r) => `${n(r.current_replicas)}/${String(r.max_replicas)}`,
      sort: (a, b) => compareNumber(a.current_replicas ?? 0, b.current_replicas ?? 0),
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
      id: 'min',
      header: 'Min',
      priority: 3,
      minWidth: 56,
      align: 'end',
      cell: (r) => n(r.min_replicas),
      text: (r) => n(r.min_replicas),
      sort: (a, b) => compareNumber(a.min_replicas ?? 1, b.min_replicas ?? 1),
    },
    {
      id: 'max',
      header: 'Max',
      priority: 4,
      minWidth: 56,
      align: 'end',
      cell: (r) => String(r.max_replicas),
      text: (r) => String(r.max_replicas),
      sort: (a, b) => compareNumber(a.max_replicas, b.max_replicas),
    },
  ],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: (r) => ({
    section: SECTION,
    kind: 'HorizontalPodAutoscaler',
    namespace: r.namespace,
    name: r.name,
  }),
  refresh: () => ({ type: 'RefreshHorizontalPodAutoscalers' }),
  read: (ref) => [
    {
      type: 'SelectHorizontalPodAutoscaler',
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
            { label: 'Min replicas', value: n(d.min_replicas), mono: true },
            { label: 'Max replicas', value: String(d.max_replicas), mono: true },
            { label: 'Current replicas', value: n(d.current_replicas), mono: true },
            ...(detail
              ? [{ label: 'Desired replicas', value: String(detail.desired_replicas), mono: true }]
              : []),
          ],
        }
      },
    },
    {
      id: 'metrics',
      title: 'Metric targets',
      render: ({ detail }) => {
        if (!detail) return null
        const ms = hpaMetrics(detail.raw_json)
        return ms.length === 0
          ? { type: 'text', text: 'No metrics configured.' }
          : {
              type: 'list',
              items: ms.map((m, i) => (
                <span
                  key={`${m.label}-${String(i)}`}
                  className="font-mono"
                  data-testid="hpa-metric"
                >
                  {m.label} — current {m.current} / target {m.target}
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
    ...labelSections<HorizontalPodAutoscalerSummary, HorizontalPodAutoscalerDetails>(),
  ],
  actions: [
    {
      ...deleteAction((namespace, name) => ({
        type: 'DeleteHorizontalPodAutoscaler',
        data: { namespace, name },
      })),
      accessCheck: (row) => ({
        verb: 'delete',
        group: 'autoscaling',
        resource: 'horizontalpodautoscalers',
        namespace: row.namespace,
      }),
    },
  ],
  selectListEvent: (e) => (e.type === 'HorizontalPodAutoscalersUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'HorizontalPodAutoscalerDetailsUpdated'
      ? (d.data as HorizontalPodAutoscalerDetails)
      : undefined,
  rawJson: (d) => d.raw_json,
}
