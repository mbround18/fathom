/** RuntimeClasses (batch D, cluster-scoped): handler, overhead (from the manifest), scheduling. */
import type { RuntimeClassDetails, RuntimeClassSummary } from '@/protocol/types'

import { labelSections, metadataSection, obj, parseJson } from './configCommon'
import { compareText } from './search'
import type { ActionDescriptor, ResourceDescriptor } from './types'

const SECTION = 'config/runtimeclasses'

export function runtimeOverhead(raw: string): string {
  const pf = obj(obj(parseJson(raw).overhead).podFixed)
  const parts = Object.entries(pf).map(([k, v]) => `${k}: ${String(v)}`)
  return parts.length ? parts.join(', ') : 'None'
}

const del: ActionDescriptor<RuntimeClassSummary> = {
  id: 'delete',
  label: 'Delete',
  done: 'Deleted',
  doing: 'Deleting',
  prompt: 'confirm',
  destructive: true,
  bulk: true,
  accessCheck: () => ({
    verb: 'delete',
    group: 'node.k8s.io',
    resource: 'runtimeclasses',
    namespace: null,
  }),
  plan: (rows) =>
    rows.map((r) => ({ cmd: { type: 'DeleteRuntimeClass', data: { name: r.name } }, rows: [r] })),
}

export const runtimeclasses: ResourceDescriptor<RuntimeClassSummary, RuntimeClassDetails> = {
  section: SECTION,
  kind: 'RuntimeClass',
  label: 'Runtime Classes',
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
      id: 'handler',
      header: 'Handler',
      priority: 1,
      minWidth: 120,
      grow: 2,
      cell: (r) => r.handler,
      text: (r) => r.handler,
      sort: (a, b) => compareText(a.handler, b.handler),
    },
  ],
  rowKey: (r) => r.name,
  ref: (r) => ({ section: SECTION, kind: 'RuntimeClass', namespace: null, name: r.name }),
  refresh: () => ({ type: 'RefreshRuntimeClasses' }),
  read: (ref) => [
    { type: 'SelectRuntimeClass', data: { name: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: [
    metadataSection(false),
    {
      id: 'runtime',
      title: 'Runtime',
      render: ({ row, detail }) => {
        const handler = detail?.handler ?? row?.handler
        if (handler === undefined) return null
        return {
          type: 'fields',
          fields: [
            { label: 'Handler', value: handler, mono: true },
            ...(detail
              ? [
                  {
                    label: 'Overhead (pod fixed)',
                    value: runtimeOverhead(detail.raw_json),
                    mono: true,
                  },
                  { label: 'Scheduling', value: detail.scheduling ?? 'None', mono: true },
                ]
              : []),
          ],
        }
      },
    },
    ...labelSections<RuntimeClassSummary, RuntimeClassDetails>(),
  ],
  actions: [del],
  selectListEvent: (e) => (e.type === 'RuntimeClassesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'RuntimeClassDetailsUpdated' ? (d.data as RuntimeClassDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
