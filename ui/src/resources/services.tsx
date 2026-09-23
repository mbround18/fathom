/**
 * Services (009 prerequisite): a minimal, read-only cluster-scoped descriptor so the Service
 * inspector has a list/detail view to host the port-forward "Forward" action (spec.md US1
 * Scenario 2) — the frontend previously had no Services view at all (`RefreshServices`/
 * `SelectService` were unused). Mirrors `nodes.tsx`'s "US2 prerequisite" precedent.
 */
import { StatusBadge } from '@/components/StatusBadge'
import { selectCmd } from '@/protocol/commands'
import type { ServiceDetails, ServiceSummary } from '@/protocol/types'

import { relatedResourcesBlock } from './related'
import { compareNumber, compareStatus, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'network/services'

const serviceSections: DetailSection<ServiceSummary, ServiceDetails>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!detail && !row) return null
      const fields = [
        { label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true },
        { label: 'Namespace', value: detail?.namespace ?? row?.namespace ?? '', mono: true },
      ]
      if (detail) {
        fields.push({ label: 'Created', value: detail.creation_timestamp, mono: true })
      }
      return { type: 'fields', fields }
    },
  },
  {
    id: 'status',
    title: 'Service',
    render: ({ row, detail }) =>
      row
        ? {
            type: 'fields',
            fields: [
              { label: 'Type', value: row.service_type, mono: true },
              { label: 'Cluster IP', value: row.cluster_ip, mono: true },
              {
                label: 'External IPs',
                value: row.external_ips.join(', ') || '—',
                mono: true,
              },
              ...(detail?.external_name
                ? [{ label: 'External name', value: detail.external_name, mono: true }]
                : []),
              ...(detail?.load_balancer_ip
                ? [{ label: 'Load balancer IP', value: detail.load_balancer_ip, mono: true }]
                : []),
            ],
          }
        : null,
  },
  {
    id: 'spec',
    title: 'Spec',
    render: ({ detail }) =>
      detail
        ? {
            type: 'list',
            items: [
              ...Object.entries(detail.selector).map(([k, v]) => (
                <span key={`selector-${k}`}>
                  selector {k}={v}
                </span>
              )),
              ...detail.ports.map((p) => (
                <span key={`port-${p.name ?? ''}:${String(p.port)}`}>
                  {p.name ?? '(unnamed)'}: {String(p.port)}
                  {p.target_port ? ` → ${p.target_port}` : ''} ({p.protocol})
                  {p.node_port ? ` nodePort ${String(p.node_port)}` : ''}
                </span>
              )),
            ],
          }
        : null,
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
  {
    id: 'related',
    title: 'Related',
    render: ({ related }) => relatedResourcesBlock(related),
  },
]

export const services: ResourceDescriptor<ServiceSummary, ServiceDetails> = {
  section: SECTION,
  kind: 'Service',
  label: 'Services',
  namespaced: true,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 3,
      cell: (s) => s.name,
      text: (s) => s.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'status',
      header: 'Type',
      priority: 1,
      minWidth: 100,
      status: true,
      cell: (s) => <StatusBadge status={s.service_type} />,
      text: (s) => s.service_type,
      sort: (a, b) => compareStatus(a.service_type, b.service_type),
    },
    {
      id: 'clusterIp',
      header: 'Cluster IP',
      priority: 2,
      minWidth: 110,
      cell: (s) => s.cluster_ip,
      text: (s) => s.cluster_ip,
      sort: (a, b) => compareText(a.cluster_ip, b.cluster_ip),
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (s) => s.namespace,
      text: (s) => s.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'ports',
      header: 'Ports',
      priority: 3,
      minWidth: 60,
      align: 'end',
      cell: (s) => String(s.port_count),
      text: (s) => String(s.port_count),
      sort: (a, b) => compareNumber(a.port_count, b.port_count),
    },
  ],
  rowKey: (s) => `${s.namespace}/${s.name}`,
  ref: (s) => ({ section: SECTION, kind: 'Service', namespace: s.namespace, name: s.name }),
  refresh: () => ({ type: 'RefreshServices' }),
  read: (ref) => {
    const cmd = selectCmd('Service', ref.namespace ?? '', ref.name)
    return [
      ...(cmd ? [cmd] : []),
      { type: 'RefreshEvents' },
      {
        type: 'RefreshRelatedResources',
        data: { section: SECTION, kind: 'Service', namespace: ref.namespace, name: ref.name },
      },
    ]
  },
  detailSections: serviceSections,
  actions: [],
  selectListEvent: (e) => (e.type === 'ServicesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'ServiceDetailsUpdated' ? (d.data as ServiceDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
