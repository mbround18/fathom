/**
 * 011-rbac-oidc-administration US1: flags a RoleBinding/ClusterRoleBinding whose `roleRef` does
 * not resolve against any currently-known Role/ClusterRole.
 *
 * Descriptors (`resources/*.tsx`) are pure data — `ColumnDef.cell`/`DetailSection.render` have no
 * hook context — so the dangling check, which needs the live Roles/ClusterRoles lists, is done
 * here instead: a small real component embedded via JSX from a `cell()`/`render()` closure
 * (`cell` and `render` both return `ReactNode`, so this is a normal embedded component, not a
 * violation of the "descriptors are pure data" rule).
 */
import { isDanglingRoleRef } from '@/resources/rbacRoleRef'
import { listKey, useStore } from '@/store'

export interface DanglingRoleRefBadgeProps {
  roleRef: string
  /** The binding's own namespace (RoleBinding) — omit/null for a ClusterRoleBinding. */
  namespace?: string | null
}

/**
 * Renders nothing when `roleRef` resolves; when it does not, a small inline warning badge with
 * `data-testid="rbac-dangling-badge"` so it exists in the DOM exactly when dangling.
 */
export function DanglingRoleRefBadge({ roleRef, namespace }: DanglingRoleRefBadgeProps) {
  const ctx = useStore((s) => s.session.activeContext)
  const roleNames = useStore((s) => {
    if (!namespace) return undefined
    const list = s.lists[listKey(ctx, 'access/roles', 'Namespaced', null)]
    if (!list) return undefined
    const names = new Set<string>()
    for (const r of list.rows) {
      if (typeof r === 'string' || !('namespace' in r) || !('name' in r)) continue
      if (r.namespace === namespace) names.add(r.name)
    }
    return names
  })
  const clusterRoleNames = useStore((s) => {
    const list = s.lists[listKey(ctx, 'access/clusterroles', 'Cluster', null)]
    const names = new Set<string>()
    if (!list) return names
    for (const r of list.rows) {
      if (typeof r === 'string' || !('name' in r)) continue
      names.add(r.name)
    }
    return names
  })

  const dangling = isDanglingRoleRef(roleRef, {
    ...(roleNames ? { roleNames } : {}),
    clusterRoleNames,
  })
  if (!dangling) return null

  return (
    <span
      data-testid="rbac-dangling-badge"
      title={`roleRef "${roleRef}" does not match any known Role or ClusterRole`}
      className="inline-flex shrink-0 items-center gap-1 border border-status-warn/60 bg-status-warn/10 px-1.5 py-0.5 text-[0.68rem] font-semibold tracking-wide text-status-warn uppercase"
    >
      <span aria-hidden="true">▲</span>
      Dangling
    </span>
  )
}
