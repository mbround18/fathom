/**
 * ClusterRoles (011-rbac-oidc-administration US1): read-only browsing of cluster-scoped
 * `ClusterRole` objects. Mirrors `nodes.tsx`'s minimal cluster-scoped descriptor shape.
 */
import type { ClusterRoleDetails, ClusterRoleSummary } from '@/protocol/types'

import { ruleTableItems } from './roles'
import { compareNumber, compareText } from './search'
import type { ActionDescriptor, DetailSection, ResourceDescriptor } from './types'

const SECTION = 'access/clusterroles'

const clusterRoleSections: DetailSection<ClusterRoleSummary, ClusterRoleDetails>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!detail && !row) return null
      const fields = [{ label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true }]
      if (detail) fields.push({ label: 'Created', value: detail.creation_timestamp, mono: true })
      return { type: 'fields', fields }
    },
  },
  {
    id: 'rules',
    title: 'Rules',
    render: ({ detail }) => (detail ? { type: 'list', items: ruleTableItems(detail.rules) } : null),
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

/**
 * `deleteAction` (actions.ts) requires `Named` (`{namespace,name}`), which `ClusterRoleSummary`
 * does not have (cluster-scoped), so this is hand-written per its `ActionDescriptor` contract.
 */
const deleteClusterRole: ActionDescriptor<ClusterRoleSummary> = {
  id: 'delete',
  label: 'Delete',
  done: 'Deleted',
  doing: 'Deleting',
  prompt: 'confirm',
  destructive: true,
  bulk: true,
  plan: (rows) =>
    rows.map((r) => ({
      cmd: { type: 'DeleteClusterRole', data: { name: r.name } },
      rows: [r],
    })),
}

export const clusterroles: ResourceDescriptor<ClusterRoleSummary, ClusterRoleDetails> = {
  section: SECTION,
  kind: 'ClusterRole',
  label: 'Cluster Roles',
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
      id: 'rulesCount',
      header: 'Rules',
      priority: 2,
      minWidth: 70,
      align: 'end',
      cell: (r) => String(r.rules_count),
      text: (r) => String(r.rules_count),
      sort: (a, b) => compareNumber(a.rules_count, b.rules_count),
    },
  ],
  rowKey: (r) => r.name,
  ref: (r) => ({ section: SECTION, kind: 'ClusterRole', namespace: null, name: r.name }),
  refresh: () => ({ type: 'RefreshClusterRoles' }),
  read: (ref) => [
    { type: 'SelectClusterRole', data: { name: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: clusterRoleSections,
  actions: [deleteClusterRole],
  selectListEvent: (e) => (e.type === 'ClusterRolesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'ClusterRoleDetailsUpdated' ? (d.data as ClusterRoleDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
