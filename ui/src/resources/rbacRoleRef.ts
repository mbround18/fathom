/**
 * 011-rbac-oidc-administration US1: dangling `roleRef` detection.
 *
 * A `RoleBinding.roleRef` may point at a `Role` in its own namespace *or* a `ClusterRole`
 * (`rbac.authorization.k8s.io` semantics); a `ClusterRoleBinding.roleRef` may only ever point at
 * a `ClusterRole`. Both `RoleBindingSummary`/`RoleBindingDetails` and
 * `ClusterRoleBindingSummary`/`ClusterRoleBindingDetails` carry `role_ref` as a plain string (the
 * referenced object's `name`; there is no kind discriminator on the wire), so "dangling" means
 * "does not match any name in the relevant set(s) of currently-known Roles/ClusterRoles".
 *
 * Pure and side-effect free so `rolebindings.tsx`, `clusterrolebindings.tsx`, the
 * `DanglingRoleRefBadge` component, and this file's own unit test all share one definition.
 */

export interface DanglingRoleRefOptions {
  /** Names of Roles in the binding's own namespace (RoleBinding only; omit for ClusterRoleBinding). */
  roleNames?: ReadonlySet<string>
  /** Names of ClusterRoles cluster-wide. Always checked. */
  clusterRoleNames: ReadonlySet<string>
}

/**
 * True when `roleRef` resolves against neither `roleNames` nor `clusterRoleNames`.
 *
 * - RoleBinding: pass both `roleNames` (its own namespace's Roles) and `clusterRoleNames` — the
 *   ref resolves if it matches either.
 * - ClusterRoleBinding: pass only `clusterRoleNames` (omit `roleNames`) — the ref only ever
 *   resolves against ClusterRoles.
 */
export function isDanglingRoleRef(roleRef: string, options: DanglingRoleRefOptions): boolean {
  if (options.roleNames?.has(roleRef)) return false
  if (options.clusterRoleNames.has(roleRef)) return false
  return true
}
