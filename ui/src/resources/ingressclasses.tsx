/** IngressClasses (034 batch B): cluster-scoped; controller, default flag, parameters. */
import { StatusBadge } from '@/components/StatusBadge'
import { selectCmd } from '@/protocol/commands'
import type { IngressClassDetails, IngressClassSummary } from '@/protocol/types'

import { compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'network/ingressclasses'
export const DEFAULT_CLASS_ANNOTATION = 'ingressclass.kubernetes.io/is-default-class'

export const isDefaultClass = (d: IngressClassDetails | undefined): boolean =>
  d?.annotations[DEFAULT_CLASS_ANNOTATION] === 'true'

const sections: DetailSection<IngressClassSummary, IngressClassDetails>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!row && !detail) return null
      const fields = [{ label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true }]
      if (detail) fields.push({ label: 'Created', value: detail.creation_timestamp, mono: true })
      return { type: 'fields', fields }
    },
  },
  {
    id: 'status',
    title: 'Status',
    render: ({ row, detail }) =>
      row || detail
        ? {
            type: 'fields',
            fields: [
              { label: 'State', value: <StatusBadge status="Active" /> },
              ...(detail
                ? [{ label: 'Default class', value: isDefaultClass(detail) ? 'Yes' : 'No' }]
                : []),
            ],
          }
        : null,
  },
  {
    id: 'spec',
    title: 'Spec',
    render: ({ row, detail }) => {
      if (!row && !detail) return null
      const p = detail?.parameters
      return {
        type: 'fields',
        fields: [
          { label: 'Controller', value: detail?.controller ?? row?.controller ?? '', mono: true },
          ...(p
            ? [
                {
                  label: 'Parameters',
                  value: `${p.api_group ? `${p.api_group}/` : ''}${p.kind}/${p.name}${p.namespace ? ` (ns ${p.namespace})` : ''}`,
                  mono: true,
                },
              ]
            : []),
        ],
      }
    },
  },
  {
    id: 'labels',
    title: 'Labels',
    render: ({ detail }) => (detail ? { type: 'map', entries: detail.labels } : null),
  },
  {
    id: 'annotations',
    title: 'Annotations',
    render: ({ detail }) => (detail ? { type: 'map', entries: detail.annotations } : null),
  },
]

export const ingressClasses: ResourceDescriptor<IngressClassSummary, IngressClassDetails> = {
  section: SECTION,
  kind: 'IngressClass',
  label: 'Ingress classes',
  namespaced: false,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 2,
      cell: (c) => c.name,
      text: (c) => c.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'status',
      header: 'Controller',
      priority: 1,
      minWidth: 180,
      grow: 3,
      cell: (c) => c.controller,
      text: (c) => c.controller,
      sort: (a, b) => compareText(a.controller, b.controller),
    },
  ],
  rowKey: (c) => c.name,
  ref: (c) => ({ section: SECTION, kind: 'IngressClass', namespace: null, name: c.name }),
  refresh: () => ({ type: 'RefreshIngressClasses' }),
  read: (ref) => {
    const cmd = selectCmd('IngressClass', '', ref.name)
    return [...(cmd ? [cmd] : []), { type: 'RefreshEvents' }]
  },
  detailSections: sections,
  actions: [],
  selectListEvent: (e) => (e.type === 'IngressClassesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'IngressClassDetailsUpdated' ? (d.data as IngressClassDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
