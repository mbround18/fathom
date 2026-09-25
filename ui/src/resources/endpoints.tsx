/** Endpoints (034 batch B): list + inspector showing ready/not-ready addresses and ports. */
import { StatusBadge } from '@/components/StatusBadge'
import { selectCmd } from '@/protocol/commands'
import type { EndpointAddress, EndpointDetails, EndpointSummary } from '@/protocol/types'

import { compareNumber, compareStatus, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'network/endpoints'

export function endpointStatus(e: EndpointSummary): string {
  if (e.ready_addresses_count === 0 && e.not_ready_addresses_count === 0) return 'Empty'
  return e.not_ready_addresses_count > 0 ? 'Degraded' : 'Ready'
}

const addr = (a: EndpointAddress) =>
  [a.ip, a.target_ref ? `→ ${a.target_ref}` : '', a.node_name ? `@ ${a.node_name}` : '']
    .filter(Boolean)
    .join(' ')

const sections: DetailSection<EndpointSummary, EndpointDetails>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!row && !detail) return null
      const fields = [
        { label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true },
        { label: 'Namespace', value: detail?.namespace ?? row?.namespace ?? '', mono: true },
      ]
      if (detail) fields.push({ label: 'Created', value: detail.creation_timestamp, mono: true })
      return { type: 'fields', fields }
    },
  },
  {
    id: 'status',
    title: 'Status',
    render: ({ row }) =>
      row
        ? {
            type: 'fields',
            fields: [
              { label: 'State', value: <StatusBadge status={endpointStatus(row)} /> },
              { label: 'Ready addresses', value: String(row.ready_addresses_count), mono: true },
              {
                label: 'Not ready addresses',
                value: String(row.not_ready_addresses_count),
                mono: true,
              },
            ],
          }
        : null,
  },
  {
    id: 'spec',
    title: 'Subsets',
    render: ({ detail }) =>
      detail
        ? {
            type: 'list',
            items: detail.subsets.map((s, i) => (
              <div key={i} className="space-y-1" data-testid="endpoint-subset">
                <div className="font-mono">
                  Ports:{' '}
                  {s.ports
                    .map((p) => `${p.name ? `${p.name} ` : ''}${String(p.port)}/${p.protocol}`)
                    .join(', ') || '—'}
                </div>
                {s.ready_addresses.map((a) => (
                  <div key={`r-${a.ip}`} className="font-mono">
                    ready {addr(a)}
                  </div>
                ))}
                {s.not_ready_addresses.map((a) => (
                  <div key={`n-${a.ip}`} className="font-mono">
                    not ready {addr(a)}
                  </div>
                ))}
              </div>
            )),
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
]

export const endpoints: ResourceDescriptor<EndpointSummary, EndpointDetails> = {
  section: SECTION,
  kind: 'Endpoint',
  label: 'Endpoints',
  namespaced: true,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 3,
      cell: (e) => e.name,
      text: (e) => e.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 100,
      status: true,
      cell: (e) => <StatusBadge status={endpointStatus(e)} />,
      text: endpointStatus,
      sort: (a, b) => compareStatus(endpointStatus(a), endpointStatus(b)),
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (e) => e.namespace,
      text: (e) => e.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'ready',
      header: 'Ready',
      priority: 3,
      minWidth: 64,
      align: 'end',
      cell: (e) => String(e.ready_addresses_count),
      text: (e) => String(e.ready_addresses_count),
      sort: (a, b) => compareNumber(a.ready_addresses_count, b.ready_addresses_count),
    },
    {
      id: 'notReady',
      header: 'Not ready',
      priority: 4,
      minWidth: 80,
      align: 'end',
      cell: (e) => String(e.not_ready_addresses_count),
      text: (e) => String(e.not_ready_addresses_count),
      sort: (a, b) => compareNumber(a.not_ready_addresses_count, b.not_ready_addresses_count),
    },
    {
      id: 'ports',
      header: 'Ports',
      priority: 4,
      minWidth: 60,
      align: 'end',
      cell: (e) => String(e.port_count),
      text: (e) => String(e.port_count),
      sort: (a, b) => compareNumber(a.port_count, b.port_count),
    },
  ],
  rowKey: (e) => `${e.namespace}/${e.name}`,
  ref: (e) => ({ section: SECTION, kind: 'Endpoint', namespace: e.namespace, name: e.name }),
  refresh: () => ({ type: 'RefreshEndpoints' }),
  read: (ref) => {
    const cmd = selectCmd('Endpoint', ref.namespace ?? '', ref.name)
    return [...(cmd ? [cmd] : []), { type: 'RefreshEvents' }]
  },
  detailSections: sections,
  actions: [],
  selectListEvent: (e) => (e.type === 'EndpointsUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'EndpointDetailsUpdated' ? (d.data as EndpointDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
