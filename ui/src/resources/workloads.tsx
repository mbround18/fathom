/**
 * Shared descriptor factory for the replica-set-style workloads (Deployments now; StatefulSets,
 * DaemonSets, ReplicaSets in T068). Ported from src/app/views/workloads.rs: the list shows
 * Namespace, Name, Ready, Updated, Available, Desired; the detail shows Status, Metadata, Pod
 * Selector, Labels, Annotations, and Pods.
 *
 * `WorkloadSummary` has no creation timestamp in protocol v1, so the planned `age` column is
 * omitted; the inspector shows it from `WorkloadDetails`.
 */
import { StatusBadge } from '@/components/StatusBadge'
import type {
  BackendCmd,
  ResourceListEvent,
  ResourceReadEvent,
  WorkloadDetails,
  WorkloadSummary,
} from '@/protocol/types'

import { formatAge } from './format'
import { compareNumber, compareStatus, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

/** `Available` when every desired replica is ready, else `Progressing` (with the counts). */
export function workloadStatus(w: WorkloadSummary): string {
  if (w.desired === 0) return 'Scaled to 0'
  return w.ready >= w.desired ? 'Available' : 'Progressing'
}

const ratio = (w: WorkloadSummary) => `${String(w.ready)}/${String(w.desired)}`

function sections(): DetailSection<WorkloadSummary, WorkloadDetails>[] {
  return [
    {
      id: 'metadata',
      title: 'Metadata',
      render: ({ row, detail }) => {
        if (!row && !detail) return null
        const fields = [
          { label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true },
          { label: 'Namespace', value: detail?.namespace ?? row?.namespace ?? '', mono: true },
        ]
        if (detail) {
          fields.push({
            label: 'Created',
            value: `${detail.creation_timestamp} (${formatAge(detail.creation_timestamp)} ago)`,
            mono: true,
          })
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
                { label: 'State', value: <StatusBadge status={workloadStatus(row)} /> },
                { label: 'Desired', value: String(row.desired), mono: true },
                { label: 'Ready', value: String(row.ready), mono: true },
                { label: 'Updated', value: String(row.updated), mono: true },
                { label: 'Available', value: String(row.available), mono: true },
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
              type: 'fields',
              fields: [
                { label: 'Pod selector', value: detail.pod_selector || '—', mono: true },
                {
                  label: `Containers (${String(detail.containers.length)})`,
                  value: detail.containers.join(', ') || '—',
                  mono: true,
                },
                {
                  label: `Ports (${String(detail.ports.length)})`,
                  value:
                    detail.ports
                      .map((p) => `${p.name ?? p.container} : ${String(p.port)} (${p.protocol})`)
                      .join(', ') || '—',
                  mono: true,
                },
              ],
            }
          : null,
    },
    {
      id: 'pods',
      title: 'Pods',
      render: ({ detail }) =>
        detail
          ? {
              type: 'list',
              items: detail.pods.map((p) => (
                <span key={p.name} className="flex min-w-0 items-center gap-2">
                  <span className="truncate font-mono">{p.name}</span>
                  <StatusBadge
                    status={p.phase}
                    label={`${p.phase} ${String(p.ready)}/${String(p.total)}`}
                  />
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
  ]
}

type ListType = ResourceListEvent['type']
type ReadType = ResourceReadEvent['type']

export interface WorkloadKind {
  section: string
  kind: string
  label: string
  listEvent: ListType
  readEvent: ReadType
  refresh: BackendCmd
  select: (namespace: string, name: string) => BackendCmd
}

export function workloadDescriptor(
  k: WorkloadKind,
): ResourceDescriptor<WorkloadSummary, WorkloadDetails> {
  return {
    section: k.section,
    kind: k.kind,
    label: k.label,
    namespaced: true,
    columns: [
      {
        id: 'name',
        header: 'Name',
        priority: 1,
        minWidth: 150,
        grow: 3,
        cell: (w) => w.name,
        text: (w) => w.name,
        sort: (a, b) => compareText(a.name, b.name),
      },
      {
        id: 'status',
        header: 'Status',
        priority: 1,
        minWidth: 116,
        status: true,
        cell: (w) => <StatusBadge status={workloadStatus(w)} />,
        text: workloadStatus,
        sort: (a, b) => compareStatus(workloadStatus(a), workloadStatus(b)),
      },
      {
        id: 'ready',
        header: 'Ready',
        priority: 2,
        minWidth: 60,
        align: 'end',
        cell: ratio,
        text: ratio,
        sort: (a, b) => compareNumber(a.ready / (a.desired || 1), b.ready / (b.desired || 1)),
      },
      {
        id: 'updated',
        header: 'Up-to-date',
        priority: 2,
        minWidth: 88,
        align: 'end',
        cell: (w) => String(w.updated),
        text: (w) => String(w.updated),
        sort: (a, b) => compareNumber(a.updated, b.updated),
      },
      {
        id: 'namespace',
        header: 'Namespace',
        priority: 2,
        minWidth: 112,
        cell: (w) => w.namespace,
        text: (w) => w.namespace,
        sort: (a, b) => compareText(a.namespace, b.namespace),
      },
      {
        id: 'available',
        header: 'Available',
        priority: 3,
        minWidth: 80,
        align: 'end',
        cell: (w) => String(w.available),
        text: (w) => String(w.available),
        sort: (a, b) => compareNumber(a.available, b.available),
      },
      {
        id: 'desired',
        header: 'Desired',
        priority: 4,
        minWidth: 72,
        align: 'end',
        cell: (w) => String(w.desired),
        text: (w) => String(w.desired),
        sort: (a, b) => compareNumber(a.desired, b.desired),
      },
    ],
    rowKey: (w) => `${w.namespace}/${w.name}`,
    ref: (w) => ({ section: k.section, kind: k.kind, namespace: w.namespace, name: w.name }),
    refresh: () => k.refresh,
    read: (ref) => [k.select(ref.namespace ?? '', ref.name), { type: 'RefreshEvents' }],
    detailSections: sections(),
    actions: [],
    selectListEvent: (e) => (e.type === k.listEvent ? (e.data as WorkloadSummary[]) : undefined),
    selectDetail: (d) => (d.eventType === k.readEvent ? (d.data as WorkloadDetails) : undefined),
    rawJson: (d) => d.raw_json,
  }
}
