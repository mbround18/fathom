/**
 * Roles (011-rbac-oidc-administration US1): read-only browsing of namespaced `Role` objects.
 * Mirrors `services.tsx`'s minimal read-only namespaced descriptor shape (no `related` section —
 * RBAC objects have no related-resources wiring).
 */
import type { PolicyRuleSummary, RoleDetails, RoleSummary } from '@/protocol/types'

import { deleteAction } from './actions'
import { compareNumber, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'access/roles'

/** `[]` is the core API group (no group prefix); render it as `(core)` rather than blank. */
function formatApiGroups(groups: readonly string[]): string {
  return groups.length === 0 ? '(core)' : groups.join(', ')
}

function formatList(items: readonly string[]): string {
  return items.length === 0 ? '—' : items.join(', ')
}

/** One rule per line: `apiGroups → resources → verbs`. Exported for `roles.test.tsx`. */
export function ruleTableItems(rules: readonly PolicyRuleSummary[]) {
  return rules.map((rule, i) => (
    <span key={i} data-testid="rbac-role-rule-row">
      <span data-testid="rbac-role-rule-api-groups">{formatApiGroups(rule.api_groups)}</span>
      {' → '}
      <span data-testid="rbac-role-rule-resources">{formatList(rule.resources)}</span>
      {' → '}
      <span data-testid="rbac-role-rule-verbs">{formatList(rule.verbs)}</span>
    </span>
  ))
}

const roleSections: DetailSection<RoleSummary, RoleDetails>[] = [
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

export const roles: ResourceDescriptor<RoleSummary, RoleDetails> = {
  section: SECTION,
  kind: 'Role',
  label: 'Roles',
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
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (r) => r.namespace,
      text: (r) => r.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
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
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: (r) => ({ section: SECTION, kind: 'Role', namespace: r.namespace, name: r.name }),
  refresh: () => ({ type: 'RefreshRoles' }),
  read: (ref) => [
    { type: 'SelectRole', data: { namespace: ref.namespace ?? '', name: ref.name } },
    { type: 'RefreshEvents' },
  ],
  detailSections: roleSections,
  actions: [deleteAction((namespace, name) => ({ type: 'DeleteRole', data: { namespace, name } }))],
  selectListEvent: (e) => (e.type === 'RolesUpdated' ? e.data : undefined),
  selectDetail: (d) => (d.eventType === 'RoleDetailsUpdated' ? (d.data as RoleDetails) : undefined),
  rawJson: (d) => d.raw_json,
}
