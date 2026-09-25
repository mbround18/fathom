/** CronJobs (batch A): list, inspector, delete, suspend/resume, trigger. */
import { StatusBadge } from '@/components/StatusBadge'
import type { CronJobDetails, CronJobSummary } from '@/protocol/types'

import { deleteAction } from './actions'
import { compareNumber, compareText } from './search'
import type { ActionDescriptor, ResourceDescriptor } from './types'

export const cronJobState = (c: Pick<CronJobSummary, 'suspend'>): string =>
  c.suspend ? 'Suspended' : 'Active'

const dash = (v: string | number | null | undefined) =>
  v === null || v === undefined || v === '' ? '—' : String(v)

/** Suspend / resume only touch rows in the opposite state, so a bulk pick never re-sends no-ops. */
const toggle = (suspend: boolean): ActionDescriptor<CronJobSummary> => ({
  id: suspend ? 'suspend' : 'resume',
  label: suspend ? 'Suspend' : 'Resume',
  done: suspend ? 'Suspended' : 'Resumed',
  doing: suspend ? 'Suspending' : 'Resuming',
  prompt: 'confirm',
  bulk: true,
  plan: (rows) =>
    rows
      .filter((r) => r.suspend !== suspend)
      .map((r) => ({
        cmd: { type: 'SuspendCronJob', data: { namespace: r.namespace, name: r.name, suspend } },
        rows: [r],
      })),
})

const trigger: ActionDescriptor<CronJobSummary> = {
  id: 'trigger',
  label: 'Trigger now',
  done: 'Triggered',
  doing: 'Triggering',
  prompt: 'confirm',
  plan: (rows) =>
    rows.map((r) => ({
      cmd: { type: 'TriggerCronJob', data: { namespace: r.namespace, name: r.name } },
      rows: [r],
    })),
}

export const cronjobs: ResourceDescriptor<CronJobSummary, CronJobDetails> = {
  section: 'workloads/cronjobs',
  kind: 'CronJob',
  label: 'CronJobs',
  namespaced: true,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 3,
      cell: (c) => c.name,
      text: (c) => c.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'status',
      header: 'State',
      priority: 1,
      minWidth: 100,
      status: true,
      cell: (c) => <StatusBadge status={cronJobState(c)} />,
      text: cronJobState,
      sort: (a, b) => compareText(cronJobState(a), cronJobState(b)),
    },
    {
      id: 'schedule',
      header: 'Schedule',
      priority: 2,
      minWidth: 112,
      cell: (c) => <span className="font-mono">{c.schedule}</span>,
      text: (c) => c.schedule,
      sort: (a, b) => compareText(a.schedule, b.schedule),
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (c) => c.namespace,
      text: (c) => c.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'active',
      header: 'Active',
      priority: 3,
      minWidth: 64,
      align: 'end',
      cell: (c) => String(c.active),
      text: (c) => String(c.active),
      sort: (a, b) => compareNumber(a.active, b.active),
    },
    {
      id: 'age',
      header: 'Age',
      priority: 3,
      minWidth: 64,
      align: 'end',
      cell: (c) => c.age,
      text: (c) => c.age,
    },
    {
      id: 'last',
      header: 'Last schedule',
      priority: 4,
      minWidth: 112,
      cell: (c) => dash(c.last_schedule),
      text: (c) => dash(c.last_schedule),
    },
  ],
  rowKey: (c) => `${c.namespace}/${c.name}`,
  ref: (c) => ({
    section: 'workloads/cronjobs',
    kind: 'CronJob',
    namespace: c.namespace,
    name: c.name,
  }),
  refresh: () => ({ type: 'RefreshCronJobs' }),
  read: (ref) => [
    { type: 'SelectCronJob', data: { namespace: ref.namespace ?? '', name: ref.name } },
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
            { label: 'State', value: <StatusBadge status={cronJobState(src)} /> },
            { label: 'Schedule', value: src.schedule, mono: true },
            { label: 'Active jobs', value: String(src.active), mono: true },
            {
              label: 'Last schedule',
              value: dash(detail?.last_schedule_time ?? row?.last_schedule),
              mono: true,
            },
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
                { label: 'Concurrency', value: dash(detail.concurrency_policy), mono: true },
                {
                  label: 'Starting deadline (s)',
                  value: dash(detail.starting_deadline_seconds),
                  mono: true,
                },
                {
                  label: 'Successful history',
                  value: dash(detail.successful_jobs_history_limit),
                  mono: true,
                },
                {
                  label: 'Failed history',
                  value: dash(detail.failed_jobs_history_limit),
                  mono: true,
                },
              ],
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
    trigger,
    toggle(true),
    toggle(false),
    deleteAction<CronJobSummary>((namespace, name) => ({
      type: 'DeleteCronJob',
      data: { namespace, name },
    })),
  ],
  selectListEvent: (e) => (e.type === 'CronJobsUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'CronJobDetailsUpdated' ? (d.data as CronJobDetails) : undefined,
}
