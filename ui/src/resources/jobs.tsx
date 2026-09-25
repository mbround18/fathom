/** Jobs (batch A): list, inspector (overview + conditions), delete. */
import { StatusBadge } from '@/components/StatusBadge'
import type { JobDetails, JobSummary } from '@/protocol/types'

import { deleteAction } from './actions'
import { compareNumber, compareStatus, compareText } from './search'
import type { ResourceDescriptor } from './types'

export function jobStatus(j: Pick<JobSummary, 'active' | 'succeeded' | 'failed'>): string {
  if (j.failed > 0 && j.active === 0 && j.succeeded === 0) return 'Failed'
  if (j.active > 0) return 'Running'
  if (j.succeeded > 0) return 'Complete'
  return 'Pending'
}

const dash = (v: string | null | undefined) => (v && v.length > 0 ? v : '—')

export const jobs: ResourceDescriptor<JobSummary, JobDetails> = {
  section: 'workloads/jobs',
  kind: 'Job',
  label: 'Jobs',
  namespaced: true,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 3,
      cell: (j) => j.name,
      text: (j) => j.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 108,
      status: true,
      cell: (j) => <StatusBadge status={jobStatus(j)} />,
      text: jobStatus,
      sort: (a, b) => compareStatus(jobStatus(a), jobStatus(b)),
    },
    {
      id: 'completions',
      header: 'Completions',
      priority: 2,
      minWidth: 96,
      align: 'end',
      cell: (j) => j.completions,
      text: (j) => j.completions,
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (j) => j.namespace,
      text: (j) => j.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'duration',
      header: 'Duration',
      priority: 3,
      minWidth: 80,
      align: 'end',
      cell: (j) => j.duration,
      text: (j) => j.duration,
    },
    {
      id: 'age',
      header: 'Age',
      priority: 3,
      minWidth: 64,
      align: 'end',
      cell: (j) => j.age,
      text: (j) => j.age,
    },
    {
      id: 'failed',
      header: 'Failed',
      priority: 4,
      minWidth: 64,
      align: 'end',
      cell: (j) => String(j.failed),
      text: (j) => String(j.failed),
      sort: (a, b) => compareNumber(a.failed, b.failed),
    },
  ],
  rowKey: (j) => `${j.namespace}/${j.name}`,
  ref: (j) => ({ section: 'workloads/jobs', kind: 'Job', namespace: j.namespace, name: j.name }),
  refresh: () => ({ type: 'RefreshJobs' }),
  read: (ref) => [
    { type: 'SelectJob', data: { namespace: ref.namespace ?? '', name: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: [
    {
      id: 'metadata',
      title: 'Metadata',
      render: ({ row, detail }) =>
        row || detail
          ? {
              type: 'fields',
              fields: [
                { label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true },
                {
                  label: 'Namespace',
                  value: detail?.namespace ?? row?.namespace ?? '',
                  mono: true,
                },
                ...(row ? [{ label: 'Age', value: row.age, mono: true }] : []),
              ],
            }
          : null,
    },
    {
      id: 'status',
      title: 'Status',
      render: ({ row, detail }) => {
        const src = detail ?? row
        if (!src) return null
        return {
          type: 'fields',
          fields: [
            { label: 'State', value: <StatusBadge status={jobStatus(src)} /> },
            { label: 'Active', value: String(src.active), mono: true },
            { label: 'Succeeded', value: String(src.succeeded), mono: true },
            { label: 'Failed', value: String(src.failed), mono: true },
            ...(detail
              ? [
                  { label: 'Started', value: dash(detail.start_time), mono: true },
                  { label: 'Completed', value: dash(detail.completion_time), mono: true },
                  { label: 'Duration', value: dash(detail.duration), mono: true },
                ]
              : []),
          ],
        }
      },
    },
    {
      id: 'spec',
      title: 'Spec',
      render: ({ detail }) =>
        detail
          ? {
              type: 'fields',
              fields: [
                { label: 'Completions', value: String(detail.completions ?? '—'), mono: true },
                { label: 'Parallelism', value: String(detail.parallelism ?? '—'), mono: true },
                { label: 'Selector', value: dash(detail.selector), mono: true },
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
                <span key={c.condition_type} className="flex flex-wrap items-center gap-2">
                  <span className="font-mono">{c.condition_type}</span>
                  <span className="font-mono">{c.status}</span>
                  {c.reason ? <span className="text-muted-foreground">{c.reason}</span> : null}
                  {c.message ? <span className="text-muted-foreground">{c.message}</span> : null}
                </span>
              )),
            }
          : null,
    },
    {
      id: 'labels',
      title: 'Labels',
      render: ({ detail }) =>
        detail ? { type: 'map', entries: Object.fromEntries(detail.labels) } : null,
    },
    {
      id: 'annotations',
      title: 'Annotations',
      render: ({ detail }) =>
        detail ? { type: 'map', entries: Object.fromEntries(detail.annotations) } : null,
    },
  ],
  actions: [
    deleteAction<JobSummary>((namespace, name) => ({
      type: 'DeleteJob',
      data: { namespace, name },
    })),
  ],
  selectListEvent: (e) => (e.type === 'JobsUpdated' ? e.data : undefined),
  selectDetail: (d) => (d.eventType === 'JobDetailsUpdated' ? (d.data as JobDetails) : undefined),
}
