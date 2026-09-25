/**
 * Namespaces (batch E): cluster-scoped list fed by `NamespaceSummariesUpdated`. The protocol has
 * no `SelectNamespace` command, so the inspector is driven by the list row alone (no read).
 */
import { StatusBadge } from '@/components/StatusBadge'
import type { NamespaceSummary } from '@/protocol/types'

import { compareNumber, compareStatus, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'namespaces'

const sections: DetailSection<NamespaceSummary>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row }) =>
      row
        ? {
            type: 'fields',
            fields: [
              { label: 'Name', value: row.name, mono: true },
              { label: 'Status', value: <StatusBadge status={row.status} /> },
              { label: 'Created', value: row.creation_timestamp || '—', mono: true },
              { label: 'Pods', value: String(row.pod_count), mono: true },
              { label: 'Deployments', value: String(row.deployment_count), mono: true },
            ],
          }
        : null,
  },
  ...(['labels', 'annotations'] as const).map((id): DetailSection<NamespaceSummary> => ({
    id,
    title: id === 'labels' ? 'Labels' : 'Annotations',
    render: ({ row }) =>
      row
        ? { type: 'text', text: `Not available: the backend has no namespace read command.` }
        : null,
  })),
]

export const namespaces: ResourceDescriptor<NamespaceSummary> = {
  section: SECTION,
  kind: 'Namespace',
  label: 'Namespaces',
  namespaced: false,
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
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 108,
      status: true,
      cell: (r) => <StatusBadge status={r.status} />,
      text: (r) => r.status,
      sort: (a, b) => compareStatus(a.status, b.status),
    },
    {
      id: 'pods',
      header: 'Pods',
      priority: 2,
      minWidth: 70,
      align: 'end',
      cell: (r) => String(r.pod_count),
      text: (r) => String(r.pod_count),
      sort: (a, b) => compareNumber(a.pod_count, b.pod_count),
    },
    {
      id: 'deployments',
      header: 'Deployments',
      priority: 3,
      minWidth: 100,
      align: 'end',
      cell: (r) => String(r.deployment_count),
      text: (r) => String(r.deployment_count),
      sort: (a, b) => compareNumber(a.deployment_count, b.deployment_count),
    },
    {
      id: 'created',
      header: 'Created',
      priority: 4,
      minWidth: 150,
      cell: (r) => r.creation_timestamp,
      text: (r) => r.creation_timestamp,
      sort: (a, b) => compareText(a.creation_timestamp, b.creation_timestamp),
    },
  ],
  rowKey: (r) => r.name,
  ref: (r) => ({ section: SECTION, kind: 'Namespace', namespace: null, name: r.name }),
  refresh: () => ({ type: 'RefreshNamespaces' }),
  detailSections: sections,
  actions: [],
  selectListEvent: (e) => (e.type === 'NamespaceSummariesUpdated' ? e.data : undefined),
  selectDetail: () => undefined,
}
