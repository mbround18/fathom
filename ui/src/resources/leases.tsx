/** Leases (batch D): holder, acquire/renew time, duration. */
import type { LeaseDetails, LeaseSummary } from '@/protocol/types'

import { deleteAction } from './actions'
import { labelSections, metadataSection } from './configCommon'
import { formatAge } from './format'
import { compareText } from './search'
import type { ResourceDescriptor } from './types'

const SECTION = 'config/leases'
const when = (ts: string | null) => (ts ? `${ts} (${formatAge(ts)} ago)` : '—')

export const leases: ResourceDescriptor<LeaseSummary, LeaseDetails> = {
  section: SECTION,
  kind: 'Lease',
  label: 'Leases',
  namespaced: true,
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
      id: 'holder',
      header: 'Holder',
      priority: 1,
      minWidth: 140,
      grow: 3,
      cell: (r) => r.holder_identity ?? '—',
      text: (r) => r.holder_identity ?? '',
      sort: (a, b) => compareText(a.holder_identity ?? '', b.holder_identity ?? ''),
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (r) => r.namespace,
      text: (r) => r.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'renew',
      header: 'Last renewed',
      priority: 3,
      minWidth: 100,
      cell: (r) => (r.renew_time ? `${formatAge(r.renew_time)} ago` : '—'),
      text: (r) => r.renew_time ?? '',
      sort: (a, b) => compareText(a.renew_time ?? '', b.renew_time ?? ''),
    },
  ],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: (r) => ({ section: SECTION, kind: 'Lease', namespace: r.namespace, name: r.name }),
  refresh: () => ({ type: 'RefreshLeases' }),
  read: (ref) => [
    { type: 'SelectLease', data: { namespace: ref.namespace ?? '', name: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: [
    metadataSection(true),
    {
      id: 'lease',
      title: 'Lease',
      render: ({ row, detail }) => {
        if (!row && !detail) return null
        const holder = detail?.holder_identity ?? row?.holder_identity ?? null
        const renew = detail?.renew_time ?? row?.renew_time ?? null
        return {
          type: 'fields',
          fields: [
            { label: 'Holder', value: holder ?? '—', mono: true },
            { label: 'Renew time', value: when(renew), mono: true },
            ...(detail
              ? [
                  { label: 'Acquire time', value: when(detail.acquire_time), mono: true },
                  {
                    label: 'Lease duration',
                    value:
                      detail.lease_duration_seconds === null
                        ? '—'
                        : `${String(detail.lease_duration_seconds)}s`,
                    mono: true,
                  },
                ]
              : []),
          ],
        }
      },
    },
    ...labelSections<LeaseSummary, LeaseDetails>(),
  ],
  actions: [
    {
      ...deleteAction((namespace, name) => ({ type: 'DeleteLease', data: { namespace, name } })),
      accessCheck: (row) => ({
        verb: 'delete',
        group: 'coordination.k8s.io',
        resource: 'leases',
        namespace: row.namespace,
      }),
    },
  ],
  selectListEvent: (e) => (e.type === 'LeasesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'LeaseDetailsUpdated' ? (d.data as LeaseDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
