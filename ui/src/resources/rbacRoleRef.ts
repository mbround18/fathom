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
  /**
   * Names of Roles in the binding's own namespace (RoleBinding only; omit for ClusterRoleBinding).
   * `undefined` for a RoleBinding means "Roles not loaded yet".
   */
  roleNames?: ReadonlySet<string> | undefined
  /** Names of ClusterRoles cluster-wide; `undefined` = not loaded yet. */
  clusterRoleNames: ReadonlySet<string> | undefined
}

/**
 * Split the backend's `Kind/name` role_ref (`src/backend/*rolebindings.rs`). A bare name (no
 * kind) is accepted too and may resolve against either set.
 */
export function parseRoleRef(roleRef: string): { kind?: 'Role' | 'ClusterRole'; name: string } {
  const slash = roleRef.indexOf('/')
  if (slash > 0) {
    const kind = roleRef.slice(0, slash)
    if (kind === 'Role' || kind === 'ClusterRole') return { kind, name: roleRef.slice(slash + 1) }
  }
  return { name: roleRef }
}

/**
 * True only when `roleRef` provably resolves to nothing: every set it could resolve against is
 * loaded and none contains it. Unknown (a needed list not loaded yet) is not dangling.
 *
 * - RoleBinding: pass both `roleNames` (its own namespace's Roles) and `clusterRoleNames`.
 * - ClusterRoleBinding: pass only `clusterRoleNames` (omit `roleNames`) — the ref only ever
 *   resolves against ClusterRoles.
 */
export function isDanglingRoleRef(roleRef: string, options: DanglingRoleRefOptions): boolean {
  const { kind, name } = parseRoleRef(roleRef)
  const { roleNames, clusterRoleNames } = options
  if (kind === 'Role') return roleNames !== undefined && !roleNames.has(name)
  if (kind === 'ClusterRole') return clusterRoleNames !== undefined && !clusterRoleNames.has(name)
  if (roleNames?.has(name) || clusterRoleNames?.has(name)) return false
  return clusterRoleNames !== undefined && ('roleNames' in options ? roleNames !== undefined : true)
}
