/**
 * Applications (spec 004, User Story 1/2): a synthesized, read-only view grouping Helm/Flux-managed
 * resources by `(namespace, instance)`. Unlike every other descriptor, there is no single backend
 * kind to select — `SelectApplication` takes `{namespace, instance}` (not `{namespace, name}`), so
 * `read()` builds its command directly instead of using `selectCmd()` (see `commands.ts`).
 */
import { StatusBadge } from '@/components/StatusBadge'
import type { ApplicationDetails, ApplicationManagedBy, ApplicationSummary } from '@/protocol/types'
import { detailKey, useStore } from '@/store'

import { formatAge, formatMi, formatMillicores } from './format'
import { relatedResourcesBlock } from './related'
import { compareNumber, compareStatus, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'applications'

export function applicationStatus(a: ApplicationSummary): string {
  if (a.total_count === 0) return 'No workloads'
  return a.ready_count >= a.total_count ? 'Available' : 'Progressing'
}

function managedByLabel(m: ApplicationManagedBy): string {
  return m === 'LabelHeuristic' ? 'Label heuristic' : m
}

/** US2: `ApplicationMetricsUpdated` is a separate read reply, keyed under its own derived kind
 * (`readKind()` strips the trailing "Updated") — looked up here rather than threaded through
 * `DetailContext` since no other descriptor renders metrics yet to share a convention with. */
function ApplicationMetricsBlock({ namespace, instance }: { namespace: string; instance: string }) {
  const ctx = useStore((s) => s.session.activeContext)
  const metricsState = useStore(
    (s) => s.details[detailKey(ctx, 'ApplicationMetrics', namespace, instance)],
  )
  if (metricsState?.eventType !== 'ApplicationMetricsUpdated') {
    return <p className="text-sm text-muted-foreground">Loading metrics…</p>
  }
  const m = metricsState.data as {
    total_cpu_usage_percent: number | null
    total_memory_usage_bytes: number | null
    total_cpu_request_millicores: number | null
    total_memory_request_bytes: number | null
  }
  if (m.total_cpu_usage_percent === null && m.total_memory_usage_bytes === null) {
    return <p className="text-sm text-muted-foreground">Metrics unavailable</p>
  }
  return (
    <dl className="grid grid-cols-[minmax(6rem,max-content)_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
      <dt className="text-muted-foreground">CPU usage</dt>
      <dd className="font-mono text-[0.85em]">
        {m.total_cpu_usage_percent === null ? '—' : `${m.total_cpu_usage_percent.toFixed(1)}%`}
      </dd>
      <dt className="text-muted-foreground">CPU requested</dt>
      <dd className="font-mono text-[0.85em]">{formatMillicores(m.total_cpu_request_millicores)}</dd>
      <dt className="text-muted-foreground">Memory usage</dt>
      <dd className="font-mono text-[0.85em]">{formatMi(m.total_memory_usage_bytes)}</dd>
      <dt className="text-muted-foreground">Memory requested</dt>
      <dd className="font-mono text-[0.85em]">{formatMi(m.total_memory_request_bytes)}</dd>
    </dl>
  )
}

const applicationSections: DetailSection<ApplicationSummary, ApplicationDetails>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!row && !detail) return null
      const fields = [
        { label: 'Instance', value: detail?.instance ?? row?.instance ?? '', mono: true },
        { label: 'Application', value: detail?.name ?? row?.name ?? '', mono: true },
        { label: 'Namespace', value: detail?.namespace ?? row?.namespace ?? '', mono: true },
        { label: 'Managed by', value: managedByLabel(detail?.managed_by ?? row?.managed_by ?? 'Helm') },
        { label: 'Version', value: detail?.version ?? row?.version ?? '—', mono: true },
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
    id: 'spec',
    title: 'URLs',
    render: ({ detail }) => {
      if (!detail) return null
      return { type: 'fields', fields: [{ label: 'URLs', value: detail.urls.join(', ') || '—', mono: true }] }
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
              { label: 'Status', value: <StatusBadge status={applicationStatus(row)} /> },
              {
                label: 'Workloads',
                value: `${String(row.ready_count)}/${String(row.total_count)}`,
                mono: true,
              },
            ],
          }
        : null,
  },
  {
    id: 'metrics',
    title: 'Metrics',
    render: ({ detail }) =>
      detail
        ? {
            type: 'list',
            items: [
              <ApplicationMetricsBlock
                key="metrics"
                namespace={detail.namespace}
                instance={detail.instance}
              />,
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
    // `ApplicationDetails` has no annotations of its own (it is synthesized, not a single k8s
    // object) — rendered as an empty map for consistency with every other descriptor's sections.
    render: ({ detail }) => (detail ? { type: 'map', entries: {} } : null),
  },
  {
    id: 'related',
    title: 'Related',
    render: ({ detail }) => (detail ? relatedResourcesBlock(detail.related) : null),
  },
]

export const applications: ResourceDescriptor<ApplicationSummary, ApplicationDetails> = {
  section: SECTION,
  kind: 'Application',
  label: 'Applications',
  namespaced: true,
  columns: [
    {
      id: 'name',
      header: 'Application',
      priority: 1,
      minWidth: 140,
      grow: 2,
      cell: (a) => a.name,
      text: (a) => a.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'instance',
      header: 'Instance',
      priority: 2,
      minWidth: 140,
      grow: 2,
      cell: (a) => a.instance,
      text: (a) => a.instance,
      sort: (a, b) => compareText(a.instance, b.instance),
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (a) => a.namespace,
      text: (a) => a.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'managed_by',
      header: 'Managed By',
      priority: 2,
      minWidth: 100,
      cell: (a) => managedByLabel(a.managed_by),
      text: (a) => managedByLabel(a.managed_by),
      sort: (a, b) => compareText(a.managed_by, b.managed_by),
    },
    {
      id: 'version',
      header: 'Version',
      priority: 3,
      minWidth: 90,
      cell: (a) => a.version ?? '—',
      text: (a) => a.version ?? '',
      sort: (a, b) => compareText(a.version ?? '', b.version ?? ''),
    },
    {
      id: 'age',
      header: 'Age',
      priority: 3,
      minWidth: 72,
      align: 'end',
      cell: (a) => formatAge(a.creation_timestamp),
      text: (a) => formatAge(a.creation_timestamp),
      sort: (a, b) => Date.parse(a.creation_timestamp) - Date.parse(b.creation_timestamp),
    },
    {
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 108,
      status: true,
      cell: (a) => <StatusBadge status={applicationStatus(a)} />,
      text: applicationStatus,
      sort: (a, b) => compareStatus(applicationStatus(a), applicationStatus(b)),
    },
    {
      id: 'workloads',
      header: 'Workloads',
      priority: 4,
      minWidth: 88,
      align: 'end',
      cell: (a) => `${String(a.ready_count)}/${String(a.total_count)}`,
      text: (a) => `${String(a.ready_count)}/${String(a.total_count)}`,
      sort: (a, b) => compareNumber(a.ready_count, b.ready_count),
    },
  ],
  rowKey: (a) => `${a.namespace}/${a.instance}`,
  ref: (a) => ({ section: SECTION, kind: 'Application', namespace: a.namespace, name: a.instance }),
  refresh: () => ({ type: 'RefreshApplications' }),
  read: (ref) => [
    { type: 'SelectApplication', data: { namespace: ref.namespace ?? '', instance: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: applicationSections,
  actions: [],
  selectListEvent: (e) => (e.type === 'ApplicationsUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'ApplicationDetailsUpdated' ? (d.data as ApplicationDetails) : undefined,
  // Deliberately no `rawJson`: Application is synthesized (not a single live k8s object), so
  // there is no `raw_json` to show — and `rawJson` on a descriptor also turns on the YAML tab's
  // apply/edit path (`descriptor.rawJson !== undefined` -> editable in Inspector.tsx), which
  // would be a mutation against a target with no real apply semantics. Out of scope for this
  // read-only feature (ground rule: no new mutations); the YAML tab shows its loading state
  // indefinitely for Applications, a known gap — see final report.
}
