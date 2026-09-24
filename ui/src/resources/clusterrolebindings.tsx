/**
 * ClusterRoleBindings (011-rbac-oidc-administration US1): read-only browsing of cluster-scoped
 * `ClusterRoleBinding` objects, plus a dangling-`roleRef` indicator. Unlike a RoleBinding, a
 * ClusterRoleBinding's `roleRef` may only ever resolve against a ClusterRole (never a Role), so
 * `DanglingRoleRefBadge` is used without a `namespace` prop here.
 */
import { DanglingRoleRefBadge } from '@/components/rbac/DanglingRoleRefBadge'
import { OrphanedSubjectBadge } from '@/features/roles/OrphanedSubjectBadge'
import type {
  ClusterRoleBindingDetails,
  ClusterRoleBindingSummary,
  SubjectSummary,
} from '@/protocol/types'

import { compareNumber, compareText } from './search'
import type { ActionDescriptor, DetailSection, ResourceDescriptor } from './types'

const SECTION = 'access/clusterrolebindings'

function subjectItems(subjects: readonly SubjectSummary[]) {
  if (subjects.length === 0) return []
  return subjects.map((s, i) => (
    <span key={i} data-testid="rbac-clusterrolebinding-subject-row">
      {s.kind} {s.name}
      {s.namespace ? ` (${s.namespace})` : ''}
    </span>
  ))
}

const clusterRoleBindingSections: DetailSection<
  ClusterRoleBindingSummary,
  ClusterRoleBindingDetails
>[] = [
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
    id: 'roleRef',
    title: 'Role Ref',
    render: ({ row, detail }) => {
      const roleRef = detail?.role_ref ?? row?.role_ref
      if (roleRef === undefined) return null
      return {
        type: 'fields',
        fields: [
          {
            label: 'ClusterRole',
            value: (
              <span className="inline-flex items-center gap-2">
                <span className="font-mono">{roleRef}</span>
                <DanglingRoleRefBadge roleRef={roleRef} />
              </span>
            ),
          },
        ],
      }
    },
  },
  {
    id: 'subjects',
    title: 'Subjects',
    render: ({ detail }) =>
      detail ? { type: 'list', items: subjectItems(detail.subjects) } : null,
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
 * `deleteAction` (actions.ts) requires `Named` (`{namespace,name}`), which
 * `ClusterRoleBindingSummary` does not have (cluster-scoped), so this is hand-written per its
 * `ActionDescriptor` contract.
 */
const deleteClusterRoleBinding: ActionDescriptor<ClusterRoleBindingSummary> = {
  id: 'delete',
  label: 'Delete',
  done: 'Deleted',
  doing: 'Deleting',
  prompt: 'confirm',
  destructive: true,
  bulk: true,
  plan: (rows) =>
    rows.map((r) => ({
      cmd: { type: 'DeleteClusterRoleBinding', data: { name: r.name } },
      rows: [r],
    })),
}

export const clusterrolebindings: ResourceDescriptor<
  ClusterRoleBindingSummary,
  ClusterRoleBindingDetails
> = {
  section: SECTION,
  kind: 'ClusterRoleBinding',
  label: 'Cluster Role Bindings',
  namespaced: false,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 3,
      cell: (r) => (
        <span className="inline-flex min-w-0 items-center gap-1.5">
          <span className="truncate">{r.name}</span>
          <OrphanedSubjectBadge binding={`ClusterRoleBinding/${r.name}`} />
        </span>
      ),
      text: (r) => r.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'roleRef',
      header: 'Role Ref',
      priority: 2,
      minWidth: 160,
      cell: (r) => (
        <span className="inline-flex min-w-0 items-center gap-1.5">
          <span className="truncate">{r.role_ref}</span>
          <DanglingRoleRefBadge roleRef={r.role_ref} />
        </span>
      ),
      text: (r) => r.role_ref,
      sort: (a, b) => compareText(a.role_ref, b.role_ref),
    },
    {
      id: 'subjectsCount',
      header: 'Subjects',
      priority: 3,
      minWidth: 80,
      align: 'end',
      cell: (r) => String(r.subjects_count),
      text: (r) => String(r.subjects_count),
      sort: (a, b) => compareNumber(a.subjects_count, b.subjects_count),
    },
  ],
  rowKey: (r) => r.name,
  ref: (r) => ({ section: SECTION, kind: 'ClusterRoleBinding', namespace: null, name: r.name }),
  refresh: () => ({ type: 'RefreshClusterRoleBindings' }),
  read: (ref) => [
    { type: 'SelectClusterRoleBinding', data: { name: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: clusterRoleBindingSections,
  actions: [deleteClusterRoleBinding],
  selectListEvent: (e) => (e.type === 'ClusterRoleBindingsUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'ClusterRoleBindingDetailsUpdated'
      ? (d.data as ClusterRoleBindingDetails)
      : undefined,
  rawJson: (d) => d.raw_json,
}
