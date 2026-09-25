/** Batch C storage kinds: PersistentVolumes, PersistentVolumeClaims, StorageClasses. */
import { StatusBadge } from '@/components/StatusBadge'
import type {
  PersistentVolumeClaimDetails,
  PersistentVolumeClaimSummary,
  PersistentVolumeDetails,
  PersistentVolumeSummary,
  StorageClassDetails,
  StorageClassSummary,
} from '@/protocol/types'

import {
  clusterRef,
  gatedClusterDelete,
  gatedDelete,
  labelSections,
  metadataSection,
  nameColumn,
  namespaceColumn,
} from './batchCCommon'
import { compareStatus, compareText } from './search'
import type { ResourceDescriptor } from './types'

const dash = (v: string | null | undefined) => (v && v.length > 0 ? v : '—')
const yn = (b: boolean | null | undefined) => (b === null || b === undefined ? '—' : b ? 'yes' : 'no')

const PV = 'storage/persistentvolumes'
const PVC = 'storage/persistentvolumeclaims'
const SC = 'storage/storageclasses'

export const persistentVolumes: ResourceDescriptor<PersistentVolumeSummary, PersistentVolumeDetails> = {
  section: PV,
  kind: 'PersistentVolume',
  label: 'Persistent Volumes',
  namespaced: false,
  columns: [
    nameColumn(),
    {
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 90,
      status: true,
      cell: (r) => <StatusBadge status={r.status} />,
      text: (r) => r.status,
      sort: (a, b) => compareStatus(a.status, b.status),
    },
    {
      id: 'capacity',
      header: 'Capacity',
      priority: 2,
      minWidth: 80,
      cell: (r) => r.capacity,
      text: (r) => r.capacity,
    },
    {
      id: 'claim',
      header: 'Claim',
      priority: 2,
      minWidth: 120,
      cell: (r) => dash(r.claim_ref),
      text: (r) => dash(r.claim_ref),
      sort: (a, b) => compareText(a.claim_ref ?? '', b.claim_ref ?? ''),
    },
    {
      id: 'storageClass',
      header: 'Storage class',
      priority: 3,
      minWidth: 110,
      cell: (r) => dash(r.storage_class),
      text: (r) => dash(r.storage_class),
    },
    {
      id: 'reclaim',
      header: 'Reclaim',
      priority: 3,
      minWidth: 80,
      cell: (r) => r.reclaim_policy,
      text: (r) => r.reclaim_policy,
    },
    {
      id: 'access',
      header: 'Access modes',
      priority: 4,
      minWidth: 110,
      cell: (r) => r.access_modes.join(', '),
      text: (r) => r.access_modes.join(', '),
    },
  ],
  rowKey: (r) => r.name,
  ref: clusterRef(PV, 'PersistentVolume'),
  refresh: () => ({ type: 'RefreshPersistentVolumes' }),
  read: (ref) => [
    { type: 'SelectPersistentVolume', data: { name: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: [
    metadataSection(false),
    {
      id: 'status',
      title: 'Status',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return {
          type: 'fields',
          fields: [
            { label: 'Phase', value: <StatusBadge status={d.status} /> },
            { label: 'Claim', value: dash(d.claim_ref), mono: true },
          ],
        }
      },
    },
    {
      id: 'spec',
      title: 'Spec',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return {
          type: 'fields',
          fields: [
            { label: 'Capacity', value: d.capacity, mono: true },
            { label: 'Access modes', value: d.access_modes.join(', ') || '—', mono: true },
            { label: 'Reclaim policy', value: d.reclaim_policy, mono: true },
            { label: 'Storage class', value: dash(d.storage_class), mono: true },
            ...(detail
              ? [
                  { label: 'Volume mode', value: dash(detail.volume_mode), mono: true },
                  { label: 'CSI driver', value: dash(detail.csi_driver), mono: true },
                ]
              : []),
          ],
        }
      },
    },
    ...labelSections(),
  ],
  actions: [
    gatedClusterDelete((name) => ({ type: 'DeletePersistentVolume', data: { name } }), 'persistentvolumes'),
  ],
  selectListEvent: (e) => (e.type === 'PersistentVolumesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'PersistentVolumeDetailsUpdated' ? (d.data as PersistentVolumeDetails) : undefined,
  rawJson: (d) => d.raw_json,
}

export const persistentVolumeClaims: ResourceDescriptor<
  PersistentVolumeClaimSummary,
  PersistentVolumeClaimDetails
> = {
  section: PVC,
  kind: 'PersistentVolumeClaim',
  label: 'Persistent Volume Claims',
  namespaced: true,
  columns: [
    nameColumn(),
    {
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 90,
      status: true,
      cell: (r) => <StatusBadge status={r.status} />,
      text: (r) => r.status,
      sort: (a, b) => compareStatus(a.status, b.status),
    },
    namespaceColumn(),
    {
      id: 'volume',
      header: 'Volume',
      priority: 2,
      minWidth: 120,
      cell: (r) => dash(r.volume_name),
      text: (r) => dash(r.volume_name),
    },
    {
      id: 'capacity',
      header: 'Capacity',
      priority: 3,
      minWidth: 80,
      cell: (r) => dash(r.capacity),
      text: (r) => dash(r.capacity),
    },
    {
      id: 'storageClass',
      header: 'Storage class',
      priority: 3,
      minWidth: 110,
      cell: (r) => dash(r.storage_class),
      text: (r) => dash(r.storage_class),
    },
    {
      id: 'access',
      header: 'Access modes',
      priority: 4,
      minWidth: 110,
      cell: (r) => r.access_modes.join(', '),
      text: (r) => r.access_modes.join(', '),
    },
  ],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: (r) => ({ section: PVC, kind: 'PersistentVolumeClaim', namespace: r.namespace, name: r.name }),
  refresh: () => ({ type: 'RefreshPersistentVolumeClaims' }),
  read: (ref) => [
    {
      type: 'SelectPersistentVolumeClaim',
      data: { namespace: ref.namespace ?? '', name: ref.name },
    },
    { type: 'RefreshEvents' },
  ],
  detailSections: [
    metadataSection(true),
    {
      id: 'status',
      title: 'Status',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return {
          type: 'fields',
          fields: [
            { label: 'Phase', value: <StatusBadge status={d.status} /> },
            { label: 'Volume', value: dash(d.volume_name), mono: true },
            { label: 'Capacity', value: dash(d.capacity), mono: true },
          ],
        }
      },
    },
    {
      id: 'spec',
      title: 'Spec',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return {
          type: 'fields',
          fields: [
            { label: 'Requested', value: dash(detail?.requested_storage), mono: true },
            { label: 'Storage class', value: dash(d.storage_class), mono: true },
            { label: 'Access modes', value: d.access_modes.join(', ') || '—', mono: true },
          ],
        }
      },
    },
    ...labelSections(),
  ],
  actions: [
    gatedDelete(
      (namespace, name) => ({ type: 'DeletePersistentVolumeClaim', data: { namespace, name } }),
      'persistentvolumeclaims',
    ),
  ],
  selectListEvent: (e) => (e.type === 'PersistentVolumeClaimsUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'PersistentVolumeClaimDetailsUpdated'
      ? (d.data as PersistentVolumeClaimDetails)
      : undefined,
  rawJson: (d) => d.raw_json,
}

export const storageClasses: ResourceDescriptor<StorageClassSummary, StorageClassDetails> = {
  section: SC,
  kind: 'StorageClass',
  label: 'Storage Classes',
  namespaced: false,
  columns: [
    nameColumn(),
    {
      id: 'status',
      header: 'Default',
      priority: 1,
      minWidth: 80,
      status: true,
      cell: (r) => (r.is_default ? <StatusBadge status="Default" /> : '—'),
      text: (r) => (r.is_default ? 'Default' : '—'),
      sort: (a, b) => Number(b.is_default) - Number(a.is_default),
    },
    {
      id: 'provisioner',
      header: 'Provisioner',
      priority: 2,
      minWidth: 150,
      grow: 2,
      cell: (r) => r.provisioner,
      text: (r) => r.provisioner,
      sort: (a, b) => compareText(a.provisioner, b.provisioner),
    },
    {
      id: 'reclaim',
      header: 'Reclaim',
      priority: 3,
      minWidth: 80,
      cell: (r) => dash(r.reclaim_policy),
      text: (r) => dash(r.reclaim_policy),
    },
    {
      id: 'binding',
      header: 'Binding mode',
      priority: 4,
      minWidth: 130,
      cell: (r) => dash(r.volume_binding_mode),
      text: (r) => dash(r.volume_binding_mode),
    },
  ],
  rowKey: (r) => r.name,
  ref: clusterRef(SC, 'StorageClass'),
  refresh: () => ({ type: 'RefreshStorageClasses' }),
  read: (ref) => [
    { type: 'SelectStorageClass', data: { name: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: [
    metadataSection(false),
    {
      id: 'status',
      title: 'Status',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return { type: 'fields', fields: [{ label: 'Default class', value: yn(d.is_default) }] }
      },
    },
    {
      id: 'spec',
      title: 'Spec',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return {
          type: 'fields',
          fields: [
            { label: 'Provisioner', value: d.provisioner, mono: true },
            { label: 'Reclaim policy', value: dash(d.reclaim_policy), mono: true },
            { label: 'Binding mode', value: dash(d.volume_binding_mode), mono: true },
            ...(detail
              ? [{ label: 'Volume expansion', value: yn(detail.allow_volume_expansion) }]
              : []),
          ],
        }
      },
    },
    {
      id: 'parameters',
      title: 'Parameters',
      render: ({ detail }) => (detail ? { type: 'map', entries: detail.parameters } : null),
    },
    ...labelSections(),
  ],
  actions: [
    gatedClusterDelete(
      (name) => ({ type: 'DeleteStorageClass', data: { name } }),
      'storageclasses',
      'storage.k8s.io',
    ),
  ],
  selectListEvent: (e) => (e.type === 'StorageClassesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'StorageClassDetailsUpdated' ? (d.data as StorageClassDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
