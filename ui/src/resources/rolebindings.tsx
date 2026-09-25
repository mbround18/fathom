/**
 * RoleBindings (011-rbac-oidc-administration US1): read-only browsing of namespaced
 * `RoleBinding` objects, plus a dangling-`roleRef` indicator (`DanglingRoleRefBadge`) — a
 * RoleBinding's `roleRef` may resolve against either a Role in its own namespace or a
 * ClusterRole. "New RoleBinding" creation is out of scope here (a separate `RoleBindingForm.tsx`
 * + entry-point wiring lands from another workstream); this descriptor only lists the read-only
 * browse + delete action.
 */
import { DanglingRoleRefBadge } from '@/components/rbac/DanglingRoleRefBadge'
import { OrphanedSubjectBadge } from '@/features/roles/OrphanedSubjectBadge'
import type { RoleBindingDetails, RoleBindingSummary, SubjectSummary } from '@/protocol/types'

import { deleteAction } from './actions'
import { compareNumber, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'access/rolebindings'

function subjectItems(subjects: readonly SubjectSummary[]) {
  if (subjects.length === 0) return []
  return subjects.map((s, i) => (
    <span key={i} data-testid="rbac-rolebinding-subject-row">
      {s.kind} {s.name}
      {s.namespace ? ` (${s.namespace})` : ''}
    </span>
  ))
}

const roleBindingSections: DetailSection<RoleBindingSummary, RoleBindingDetails>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!detail && !row) return null
      const fields = [
        { label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true },
        { label: 'Namespace', value: detail?.namespace ?? row?.namespace ?? '', mono: true },
      ]
      if (detail) fields.push({ label: 'Created', value: detail.creation_timestamp, mono: true })
      return { type: 'fields', fields }
    },
  },
  {
    id: 'roleRef',
    title: 'Role Ref',
    render: ({ row, detail }) => {
      const roleRef = detail?.role_ref ?? row?.role_ref
      const namespace = detail?.namespace ?? row?.namespace
      if (roleRef === undefined) return null
      return {
        type: 'fields',
        fields: [
          {
            label: 'Role',
            value: (
              <span className="inline-flex items-center gap-2">
                <span className="font-mono">{roleRef}</span>
                <DanglingRoleRefBadge roleRef={roleRef} namespace={namespace} />
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

export const rolebindings: ResourceDescriptor<RoleBindingSummary, RoleBindingDetails> = {
  section: SECTION,
  kind: 'RoleBinding',
  label: 'Role Bindings',
  namespaced: true,
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
          <OrphanedSubjectBadge binding={`RoleBinding/${r.namespace}/${r.name}`} />
        </span>
      ),
      text: (r) => r.name,
      sort: (a, b) => compareText(a.name, b.name),
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
      id: 'roleRef',
      header: 'Role Ref',
      priority: 2,
      minWidth: 160,
      cell: (r) => (
        <span className="inline-flex min-w-0 items-center gap-1.5">
          <span className="truncate">{r.role_ref}</span>
          <DanglingRoleRefBadge roleRef={r.role_ref} namespace={r.namespace} />
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
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: (r) => ({ section: SECTION, kind: 'RoleBinding', namespace: r.namespace, name: r.name }),
  refresh: () => ({ type: 'RefreshRoleBindings' }),
  read: (ref) => [
    { type: 'SelectRoleBinding', data: { namespace: ref.namespace ?? '', name: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: roleBindingSections,
  actions: [
    {
      ...deleteAction((namespace, name) => ({ type: 'DeleteRoleBinding', data: { namespace, name } })),
      accessCheck: (row) => ({
        verb: 'delete',
        group: 'rbac.authorization.k8s.io',
        resource: 'rolebindings',
        namespace: row.namespace,
      }),
    },
  ],
  selectListEvent: (e) => (e.type === 'RoleBindingsUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'RoleBindingDetailsUpdated' ? (d.data as RoleBindingDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
