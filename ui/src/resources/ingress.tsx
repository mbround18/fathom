/** Ingresses (034 batch B): rules/hosts/backends/TLS overview, delete. */
import { StatusBadge } from '@/components/StatusBadge'
import { selectCmd } from '@/protocol/commands'
import type { IngressBackend, IngressDetails, IngressSummary } from '@/protocol/types'

import { deleteAction } from './actions'
import { compareNumber, compareStatus, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'network/ingress'

export function ingressStatus(i: IngressSummary): string {
  return i.load_balancer_ips.length > 0 ? 'Ready' : 'Pending'
}

export function backendText(b: IngressBackend): string {
  if (b.service_name) {
    return `${b.service_name}${b.service_port ? `:${String(b.service_port)}` : ''}`
  }
  if (b.resource_name) return `${b.resource_kind ?? 'Resource'}/${b.resource_name}`
  return '—'
}

const sections: DetailSection<IngressSummary, IngressDetails>[] = [
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
    render: ({ row, detail }) => {
      if (!row) return null
      const addrs = [
        ...row.load_balancer_ips,
        ...(detail?.load_balancer_ingress.flatMap((l) => (l.hostname ? [l.hostname] : [])) ?? []),
      ]
      return {
        type: 'fields',
        fields: [
          { label: 'State', value: <StatusBadge status={ingressStatus(row)} /> },
          { label: 'Class', value: row.ingress_class ?? '—', mono: true },
          { label: 'Load balancer', value: addrs.join(', ') || '—', mono: true },
        ],
      }
    },
  },
  {
    id: 'spec',
    title: 'Rules',
    render: ({ detail }) => {
      if (!detail) return null
      const items = detail.rules.flatMap((r, ri) =>
        r.paths.map((p, pi) => (
          <div key={`${String(ri)}-${String(pi)}`} className="font-mono" data-testid="ingress-rule">
            {r.host ?? '*'}
            {p.path ?? '/'} ({p.path_type}) → {backendText(p.backend)}
          </div>
        )),
      )
      if (detail.default_backend) {
        items.push(
          <div key="default" className="font-mono">
            default → {backendText(detail.default_backend)}
          </div>,
        )
      }
      if (detail.tls_hosts.length > 0) {
        items.push(
          <div key="tls" className="font-mono" data-testid="ingress-tls">
            TLS: {detail.tls_hosts.join(', ')}
          </div>,
        )
      }
      return { type: 'list', items }
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

export const ingresses: ResourceDescriptor<IngressSummary, IngressDetails> = {
  section: SECTION,
  kind: 'Ingress',
  label: 'Ingresses',
  namespaced: true,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 3,
      cell: (i) => i.name,
      text: (i) => i.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 100,
      status: true,
      cell: (i) => <StatusBadge status={ingressStatus(i)} />,
      text: ingressStatus,
      sort: (a, b) => compareStatus(ingressStatus(a), ingressStatus(b)),
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (i) => i.namespace,
      text: (i) => i.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'class',
      header: 'Class',
      priority: 3,
      minWidth: 90,
      cell: (i) => i.ingress_class ?? '—',
      text: (i) => i.ingress_class ?? '—',
      sort: (a, b) => compareText(a.ingress_class ?? '', b.ingress_class ?? ''),
    },
    {
      id: 'hosts',
      header: 'Hosts',
      priority: 3,
      minWidth: 60,
      align: 'end',
      cell: (i) => String(i.hosts_count),
      text: (i) => String(i.hosts_count),
      sort: (a, b) => compareNumber(a.hosts_count, b.hosts_count),
    },
    {
      id: 'address',
      header: 'Address',
      priority: 4,
      minWidth: 120,
      cell: (i) => i.load_balancer_ips.join(', ') || '—',
      text: (i) => i.load_balancer_ips.join(', ') || '—',
    },
  ],
  rowKey: (i) => `${i.namespace}/${i.name}`,
  ref: (i) => ({ section: SECTION, kind: 'Ingress', namespace: i.namespace, name: i.name }),
  refresh: () => ({ type: 'RefreshIngresses' }),
  read: (ref) => {
    const cmd = selectCmd('Ingress', ref.namespace ?? '', ref.name)
    return [...(cmd ? [cmd] : []), { type: 'RefreshEvents' }]
  },
  detailSections: sections,
  actions: [
    deleteAction<IngressSummary>((namespace, name) => ({
      type: 'DeleteIngress',
      data: { namespace, name },
    })),
  ],
  selectListEvent: (e) => (e.type === 'IngressesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'IngressDetailsUpdated' ? (d.data as IngressDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
