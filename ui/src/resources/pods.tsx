/**
 * Pods (T066). Columns and detail sections ported from src/app/views/pods.rs.
 *
 * The protocol v1 `PodSummary` has no creation timestamp or pod IP, so the planned `age` (p3)
 * and `IP` (p5) columns are omitted; the inspector shows the age from `PodDetails`.
 */
import { PodLifecyclePanels } from '@/components/PodLifecyclePanels'
import { StatusBadge } from '@/components/StatusBadge'
import { selectCmd } from '@/protocol/commands'
import type { PodDetails, PodSummary } from '@/protocol/types'

import { addEphemeralContainerAction, deleteAction } from './actions'
import { formatAge, formatMi, formatMillicores } from './format'
import { relatedResourcesBlock } from './related'
import { compareNumber, compareStatus, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'workloads/pods'

export function podStatus(p: PodSummary): string {
  return p.phase
}

const podSections: DetailSection<PodSummary, PodDetails>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!detail && !row) return null
      const ns = detail?.namespace ?? row?.namespace ?? ''
      const fields = [
        { label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true },
        { label: 'Namespace', value: ns, mono: true },
      ]
      if (detail) {
        fields.push(
          {
            label: 'Created',
            value: `${detail.creation_timestamp} (${formatAge(detail.creation_timestamp)} ago)`,
            mono: true,
          },
          {
            label: 'Owner',
            value: detail.owner ? `${detail.owner.kind} / ${detail.owner.name}` : '—',
            mono: true,
          },
        )
        if (detail.resolved_owner) {
          fields.push({
            label: 'Controlled by',
            value: `${detail.resolved_owner.kind} / ${detail.resolved_owner.name}`,
            mono: true,
          })
        }
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
              { label: 'Phase', value: <StatusBadge status={row.phase} /> },
              { label: 'Ready', value: `${String(row.ready)}/${String(row.total)}`, mono: true },
              { label: 'Restarts', value: String(row.restarts), mono: true },
              { label: 'Node', value: row.node || '—', mono: true },
            ],
          }
        : null,
  },
  {
    id: 'spec',
    title: 'Spec',
    render: ({ detail }) => {
      if (!detail) return null
      // 025-ephemeral-debug-containers: debug containers, listed apart from the spec containers.
      const debug = detail.containers_lifecycle.filter((c) => c.is_ephemeral)
      const fields = [
        {
          label: `Containers (${String(detail.containers.length)})`,
          value: detail.containers.join(', ') || '—',
          mono: true,
        },
        ...(debug.length > 0
          ? [
              {
                label: `Debug containers (${String(debug.length)})`,
                value: debug.map((c) => `${c.name} (${c.state.phase})`).join(', '),
                mono: true,
              },
            ]
          : []),
        {
          label: `Ports (${String(detail.ports.length)})`,
          value:
            detail.ports
              .map((p) => `${p.name ?? p.container} : ${String(p.port)} (${p.protocol})`)
              .join(', ') || '—',
          mono: true,
        },
      ]
      for (const r of detail.resource_specs) {
        fields.push({
          label: `${r.container} resources`,
          value: `cpu ${formatMillicores(r.cpu_request_millicores)} / ${formatMillicores(r.cpu_limit_millicores)} · mem ${formatMi(r.memory_request_bytes)} / ${formatMi(r.memory_limit_bytes)}`,
          mono: true,
        })
      }
      fields.push({
        label: 'Volumes (PVC)',
        value: detail.persistent_volume_claims.join(', ') || '—',
        mono: true,
      })
      return { type: 'fields', fields }
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
  {
    id: 'related',
    title: 'Related',
    render: ({ related }) => relatedResourcesBlock(related),
  },
  {
    // 008-pod-lifecycle-diagnostics: init containers, probes, restart history, hooks, and the
    // Pod's own Events, all sourced from `detail.containers_lifecycle` (additive — FR-018,
    // placed after `related` so existing sections/tests are unaffected).
    id: 'lifecycle',
    title: 'Lifecycle',
    render: ({ detail }) => {
      if (!detail) return null
      return {
        type: 'list',
        items: [
          <PodLifecyclePanels
            key="pod-lifecycle"
            namespace={detail.namespace}
            name={detail.name}
            containers={detail.containers_lifecycle}
          />,
        ],
      }
    },
  },
]

export const pods: ResourceDescriptor<PodSummary, PodDetails> = {
  section: SECTION,
  kind: 'Pod',
  label: 'Pods',
  namespaced: true,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 3,
      cell: (p) => p.name,
      text: (p) => p.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 108,
      status: true,
      cell: (p) => <StatusBadge status={podStatus(p)} />,
      text: podStatus,
      sort: (a, b) => compareStatus(podStatus(a), podStatus(b)),
    },
    {
      id: 'ready',
      header: 'Ready',
      priority: 2,
      minWidth: 60,
      align: 'end',
      cell: (p) => `${String(p.ready)}/${String(p.total)}`,
      text: (p) => `${String(p.ready)}/${String(p.total)}`,
      sort: (a, b) => compareNumber(a.ready / (a.total || 1), b.ready / (b.total || 1)),
    },
    {
      id: 'restarts',
      header: 'Restarts',
      priority: 2,
      minWidth: 76,
      align: 'end',
      cell: (p) => String(p.restarts),
      text: (p) => String(p.restarts),
      sort: (a, b) => compareNumber(a.restarts, b.restarts),
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (p) => p.namespace,
      text: (p) => p.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'node',
      header: 'Node',
      priority: 4,
      minWidth: 140,
      grow: 1.5,
      cell: (p) => p.node || '—',
      text: (p) => p.node,
      sort: (a, b) => compareText(a.node, b.node),
    },
  ],
  rowKey: (p) => `${p.namespace}/${p.name}`,
  ref: (p) => ({ section: SECTION, kind: 'Pod', namespace: p.namespace, name: p.name }),
  refresh: () => ({ type: 'RefreshPods' }),
  read: (ref) => {
    const cmd = selectCmd('Pod', ref.namespace ?? '', ref.name)
    return [
      ...(cmd ? [cmd] : []),
      { type: 'RefreshEvents' },
      {
        type: 'RefreshRelatedResources',
        data: { section: SECTION, kind: 'Pod', namespace: ref.namespace, name: ref.name },
      },
    ]
  },
  detailSections: podSections,
  actions: [
    deleteAction<PodSummary>(
      (namespace, name) => ({ type: 'DeletePod', data: { namespace, name } }),
      (rows) => ({
        type: 'BulkDelete',
        data: { items: rows.map((r): [string, string] => [r.namespace, r.name]) },
      }),
    ),
    addEphemeralContainerAction<PodSummary>(),
  ],
  selectListEvent: (e) => (e.type === 'PodsUpdated' ? e.data : undefined),
  selectDetail: (d) => (d.eventType === 'PodDetailsUpdated' ? (d.data as PodDetails) : undefined),
  rawJson: (d) => d.raw_json,
}
