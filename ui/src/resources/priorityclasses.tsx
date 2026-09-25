/** PriorityClasses (batch D, cluster-scoped). */
import type { PriorityClassDetails, PriorityClassSummary } from '@/protocol/types'

import { labelSections, metadataSection } from './configCommon'
import { compareNumber, compareText } from './search'
import type { ActionDescriptor, ResourceDescriptor } from './types'

const SECTION = 'config/priorityclasses'
const yn = (b: boolean) => (b ? 'Yes' : 'No')

const del: ActionDescriptor<PriorityClassSummary> = {
  id: 'delete',
  label: 'Delete',
  done: 'Deleted',
  doing: 'Deleting',
  prompt: 'confirm',
  destructive: true,
  bulk: true,
  accessCheck: () => ({
    verb: 'delete',
    group: 'scheduling.k8s.io',
    resource: 'priorityclasses',
    namespace: null,
  }),
  plan: (rows) =>
    rows.map((r) => ({ cmd: { type: 'DeletePriorityClass', data: { name: r.name } }, rows: [r] })),
}

export const priorityclasses: ResourceDescriptor<PriorityClassSummary, PriorityClassDetails> = {
  section: SECTION,
  kind: 'PriorityClass',
  label: 'Priority Classes',
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
      id: 'value',
      header: 'Value',
      priority: 1,
      minWidth: 90,
      align: 'end',
      cell: (r) => String(r.value),
      text: (r) => String(r.value),
      sort: (a, b) => compareNumber(a.value, b.value),
    },
    {
      id: 'default',
      header: 'Global default',
      priority: 2,
      minWidth: 110,
      cell: (r) => yn(r.global_default),
      text: (r) => yn(r.global_default),
      sort: (a, b) => compareNumber(Number(a.global_default), Number(b.global_default)),
    },
    {
      id: 'description',
      header: 'Description',
      priority: 3,
      minWidth: 160,
      grow: 3,
      cell: (r) => r.description ?? '—',
      text: (r) => r.description ?? '',
      sort: (a, b) => compareText(a.description ?? '', b.description ?? ''),
    },
  ],
  rowKey: (r) => r.name,
  ref: (r) => ({ section: SECTION, kind: 'PriorityClass', namespace: null, name: r.name }),
  refresh: () => ({ type: 'RefreshPriorityClasses' }),
  read: (ref) => [
    { type: 'SelectPriorityClass', data: { name: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: [
    metadataSection(false),
    {
      id: 'priority',
      title: 'Priority',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return {
          type: 'fields',
          fields: [
            { label: 'Value', value: String(d.value), mono: true },
            { label: 'Global default', value: yn(d.global_default) },
            ...(detail
              ? [
                  {
                    label: 'Preemption policy',
                    value: detail.preemption_policy ?? 'PreemptLowerPriority (default)',
                    mono: true,
                  },
                ]
              : []),
            { label: 'Description', value: d.description ?? '—' },
          ],
        }
      },
    },
    ...labelSections<PriorityClassSummary, PriorityClassDetails>(),
  ],
  actions: [del],
  selectListEvent: (e) => (e.type === 'PriorityClassesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'PriorityClassDetailsUpdated' ? (d.data as PriorityClassDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
