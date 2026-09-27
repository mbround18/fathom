/**
 * 033 US2: per-context "authenticate with OIDC" control. Kubeconfig auth is the default; opting a
 * context in reconnects it with the signed-in ID token. A rejected token is shown verbatim with a
 * pointer to the 011 OIDC config generator, which can be prefilled from the issuer.
 */
import { useStore } from '@/store'

export function ContextAuth({ only }: { only?: string } = {}) {
  const send = useStore((s) => s.send)
  const all = useStore((s) => s.session.contexts)
  // 043: the setup sign-in step shows just the picked context.
  const contexts = only === undefined ? all : all.filter((c) => c === only)
  const current = useStore((s) => s.session.activeContext)
  const issuers = useStore((s) => s.identity.config.issuers)
  const bindings = useStore((s) => s.identity.config.contexts)
  const notice = useStore((s) => s.conn.lastNotice)

  if (issuers.length === 0) {
    return (
      <p className="text-xs text-muted-foreground" data-testid="oidc-context-auth-empty">
        Add an OIDC issuer to authenticate a context with it.
      </p>
    )
  }
  const rejected = notice?.level === 'error' ? notice.message : undefined

  return (
    <div className="flex flex-col gap-2" data-testid="oidc-context-auth">
      {contexts.map((ctx) => {
        const binding = bindings.find((b) => b.context_name === ctx)
        const oidc = binding?.mode === 'oidc'
        return (
          <div
            key={ctx}
            className="flex flex-wrap items-center gap-2 text-xs"
            data-testid={`oidc-context-${ctx}`}
          >
            <span className="min-w-0 flex-1 truncate">{ctx}</span>
            <select
              aria-label={`Authentication for ${ctx}`}
              data-testid={`oidc-context-mode-${ctx}`}
              className="border border-border bg-background px-1 py-1"
              value={oidc ? (binding.issuer_id ?? '') : ''}
              onChange={(e) => {
                const issuer_id = e.target.value || null
                send({
                  type: 'SetContextAuthMode',
                  data: {
                    context: ctx,
                    mode: issuer_id ? 'oidc' : 'kubeconfig',
                    issuer_id,
                  },
                })
              }}
            >
              <option value="">Kubeconfig credentials</option>
              {issuers.map((i) => (
                <option key={i.id} value={i.id}>
                  OIDC: {i.id}
                </option>
              ))}
            </select>
          </div>
        )
      })}
      {rejected &&
        current &&
        bindings.some((b) => b.context_name === current && b.mode === 'oidc') && (
          <div
            role="alert"
            className="border-status-error/50 border p-2 text-xs"
            data-testid="oidc-context-rejected"
          >
            <p className="font-mono">{rejected}</p>
            <p className="mt-1 text-muted-foreground">
              If the API server does not trust this issuer, generate its OIDC configuration under
              Access, OIDC config (prefilled from your issuer).
            </p>
          </div>
        )}
    </div>
  )
}
