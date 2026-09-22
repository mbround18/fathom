/**
 * Nodes (US2 prerequisite): a minimal cluster-scoped descriptor so node actions
 * (cordon/uncordon/drain/taint, NodeActionsPanel) have a list/detail view to hang off — the
 * frontend previously had no Nodes view at all (SelectNode/NodeSummary were unused).
 */
import { StatusBadge } from '@/components/StatusBadge'
import { selectCmd } from '@/protocol/commands'
import type { NodeDetails, NodeSummary } from '@/protocol/types'

import { formatMi, formatMillicores } from './format'
import { relatedResourcesBlock } from './related'
import { compareNumber, compareStatus, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'nodes'

export function nodeStatus(n: NodeSummary): string {
  return n.status
}

const nodeSections: DetailSection<NodeSummary, NodeDetails>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!detail && !row) return null
      const fields = [{ label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true }]
      if (detail) {
        fields.push(
          { label: 'Roles', value: detail.roles.join(', ') || '—', mono: true },
          { label: 'Created', value: detail.creation_timestamp, mono: true },
          { label: 'OS / Arch', value: `${detail.os} / ${detail.arch}`, mono: true },
          { label: 'Kernel', value: detail.kernel_version, mono: true },
          { label: 'Container runtime', value: detail.container_runtime, mono: true },
          { label: 'Kubelet', value: detail.kubelet_version, mono: true },
        )
      }
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
              { label: 'Status', value: <StatusBadge status={nodeStatus(row)} /> },
              { label: 'Role', value: row.role, mono: true },
              {
                label: 'Pods',
                value: `${String(row.pod_count)}/${String(row.pod_capacity)}`,
                mono: true,
              },
              {
                label: 'CPU allocatable',
                value: formatMillicores(row.cpu_allocatable_millicores),
                mono: true,
              },
              {
                label: 'Memory allocatable',
                value: formatMi(row.memory_allocatable_bytes),
                mono: true,
              },
            ],
          }
        : null,
  },
  {
    id: 'spec',
    title: 'Capacity',
    render: ({ detail }) =>
      detail
        ? {
            type: 'fields',
            fields: [
              { label: 'Capacity CPU', value: detail.capacity_cpu, mono: true },
              { label: 'Capacity memory', value: detail.capacity_memory, mono: true },
              { label: 'Allocatable CPU', value: detail.allocatable_cpu, mono: true },
              { label: 'Allocatable memory', value: detail.allocatable_memory, mono: true },
            ],
          }
        : null,
  },
  {
    id: 'conditions',
    title: 'Conditions',
    render: ({ detail }) =>
      detail
        ? {
            type: 'list',
            items: detail.conditions.map((c) => (
              <span key={c.condition_type}>
                {c.condition_type}: {c.status} {c.reason ? `(${c.reason})` : ''}
              </span>
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
  {
    id: 'related',
    title: 'Related',
    render: ({ related }) => relatedResourcesBlock(related),
  },
]

export const nodes: ResourceDescriptor<NodeSummary, NodeDetails> = {
  section: SECTION,
  kind: 'Node',
  label: 'Nodes',
  namespaced: false,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 3,
      cell: (n) => n.name,
      text: (n) => n.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 108,
      status: true,
      cell: (n) => <StatusBadge status={nodeStatus(n)} />,
      text: nodeStatus,
      sort: (a, b) => compareStatus(nodeStatus(a), nodeStatus(b)),
    },
    {
      id: 'role',
      header: 'Role',
      priority: 2,
      minWidth: 100,
      cell: (n) => n.role,
      text: (n) => n.role,
      sort: (a, b) => compareText(a.role, b.role),
    },
    {
      id: 'pods',
      header: 'Pods',
      priority: 2,
      minWidth: 76,
      align: 'end',
      cell: (n) => `${String(n.pod_count)}/${String(n.pod_capacity)}`,
      text: (n) => `${String(n.pod_count)}/${String(n.pod_capacity)}`,
      sort: (a, b) => compareNumber(a.pod_count, b.pod_count),
    },
    {
      id: 'cpu',
      header: 'CPU',
      priority: 3,
      minWidth: 90,
      align: 'end',
      cell: (n) => formatMillicores(n.cpu_allocatable_millicores),
      text: (n) => formatMillicores(n.cpu_allocatable_millicores),
      sort: (a, b) => compareNumber(a.cpu_allocatable_millicores, b.cpu_allocatable_millicores),
    },
    {
      id: 'memory',
      header: 'Memory',
      priority: 3,
      minWidth: 90,
      align: 'end',
      cell: (n) => formatMi(n.memory_allocatable_bytes),
      text: (n) => formatMi(n.memory_allocatable_bytes),
      sort: (a, b) => compareNumber(a.memory_allocatable_bytes, b.memory_allocatable_bytes),
    },
  ],
  rowKey: (n) => n.name,
  ref: (n) => ({ section: SECTION, kind: 'Node', namespace: null, name: n.name }),
  refresh: () => ({ type: 'RefreshNodes' }),
  read: (ref) => {
    const cmd = selectCmd('Node', '', ref.name)
    return [
      ...(cmd ? [cmd] : []),
      { type: 'RefreshEvents' },
      {
        type: 'RefreshRelatedResources',
        data: { section: SECTION, kind: 'Node', namespace: null, name: ref.name },
      },
    ]
  },
  detailSections: nodeSections,
  actions: [],
  selectListEvent: (e) => (e.type === 'NodesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'NodeDetailsUpdated' ? (d.data as NodeDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
