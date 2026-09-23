/**
 * 011-rbac-oidc-administration: surfaces scopes (namespaces, or any other named partition) a
 * list/lookup could not fully read, plus the backend's error text verbatim (constitution I: never
 * paraphrase a backend error). Deliberately generic — used here for a degraded Roles/RoleBindings
 * list-refresh, and later by US4's namespace-access lookup, which gets its own `degraded_scopes`
 * from `NamespaceAccessResolved` — so it takes a plain scope list and message rather than
 * anything RBAC-list-specific.
 */
export interface DegradedScopeBannerProps {
  /** The scopes (namespace names, or another caller-defined label) that could not be read. */
  scopes: readonly string[]
  /** The backend's error text, shown verbatim — never paraphrased or summarized. */
  message?: string
}

export function DegradedScopeBanner({ scopes, message }: DegradedScopeBannerProps) {
  if (scopes.length === 0 && !message) return null
  return (
    <div
      data-testid="rbac-degraded-scope"
      role="alert"
      className="flex flex-col gap-1 border-b border-status-warn/50 bg-status-warn/10 px-2 py-1.5 text-xs text-status-warn"
    >
      <div className="flex items-center gap-1.5 font-semibold">
        <span aria-hidden="true">▲</span>
        <span>
          {scopes.length > 0
            ? `Incomplete for: ${scopes.join(', ')}`
            : 'Some scopes could not be fully read'}
        </span>
      </div>
      {message && <p className="text-muted-foreground">{message}</p>}
    </div>
  )
}
