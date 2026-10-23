/**
 * Cluster Settings > Cloud auth (035-cloud-auth-plugins): the detected exec plugin for the active
 * context (EKS / GKE / AKS / other), whether it is installed, a copyable install hint, and the
 * per-context exec policy. Detection never runs the plugin; the policy toggle is RBAC-gated like
 * every mutating control.
 */
import { Button } from '@companion/ui'
import { Gated } from '@/lib/capabilities'
import { useStore } from '@/store'

const PROVIDER_LABEL: Record<string, string> = {
  eks: 'Amazon EKS',
  gke: 'Google GKE',
  aks: 'Azure AKS',
  genericExec: 'Exec plugin',
  none: 'Static credentials',
}

export function CloudAuthSection() {
  const send = useStore((s) => s.send)
  const context = useStore((s) => s.session.activeContext)
  const info = useStore((s) => (context ? s.session.cloudAuth[context] : undefined))
  if (!context || !info || info.provider === 'none') return null

  const allowAll = info.exec_policy === 'allowAll'
  const setPolicy = () =>
    send({
      type: 'SetExecPolicy',
      data: { context, policy: allowAll ? 'allowKnownOnly' : 'allowAll' },
    })
  const copy = () => {
    if (info.install_hint) void navigator.clipboard.writeText(info.install_hint)
  }

  return (
    <section
      aria-label="Cloud auth"
      data-testid="cloud-auth-settings"
      className="flex flex-col gap-2 p-3"
    >
      <h3 className="font-display text-xs tracking-wider uppercase">Cloud auth</h3>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt>Provider</dt>
        <dd data-testid="cloud-auth-provider">{PROVIDER_LABEL[info.provider] ?? info.provider}</dd>
        <dt>Plugin</dt>
        <dd className="font-mono" data-testid="cloud-auth-command">
          {info.command ?? '-'}{' '}
          <span data-testid="cloud-auth-installed">
            {info.installed ? '(installed)' : '(not found on PATH)'}
          </span>
        </dd>
      </dl>
      {info.installed === false && info.install_hint && (
        <div className="flex items-center gap-2 text-xs">
          <code data-testid="cloud-auth-hint" className="font-mono break-all">
            {info.install_hint}
          </code>
          <Button type="button" size="sm" variant="outline" onClick={copy}>
            Copy
          </Button>
        </div>
      )}
      {!info.exec_policy_allowed || allowAll ? (
        <p className="text-xs text-muted-foreground">
          {allowAll
            ? 'Any exec command is allowed for this context.'
            : 'This command is blocked: only known cloud auth plugins named as a bare command (resolved from PATH) run under the default policy.'}
        </p>
      ) : null}
      <Gated verb="get" resource="namespaces">
        <Button
          type="button"
          size="sm"
          variant="outline"
          data-testid="cloud-auth-policy-toggle"
          onClick={setPolicy}
        >
          {allowAll ? 'Restrict to known plugins' : 'Allow any exec command'}
        </Button>
      </Gated>
    </section>
  )
}
