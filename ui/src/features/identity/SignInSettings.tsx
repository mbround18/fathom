/**
 * 043 US2: the sign-in controls in one place: OIDC issuers with their sign-in panels (033), how
 * each context authenticates (033 US2) and the context's exec plugin (035). Used by the setup
 * screen's optional sign-in step and by Settings › Sign-in; the header identity dialog shows the
 * same issuer and context controls.
 */
import { PlusIcon } from '@phosphor-icons/react'
import { type ReactNode, useId, useState } from 'react'

import { Button } from '@companion/ui'
import { useStore } from '@/store'

import { ContextAuth } from './ContextAuth'
import { IssuerSettings } from './IssuerSettings'

const PROVIDER_LABEL: Record<string, string> = {
  eks: 'Amazon EKS',
  gke: 'Google GKE',
  aks: 'Azure AKS',
  genericExec: 'Exec plugin',
}

/** Read-only exec-plugin status for `context` (the policy toggle stays in Cluster settings). */
export function ExecPluginStatus({ context }: { context: string }) {
  const info = useStore((s) => s.session.cloudAuth[context])
  if (!info) return null
  if (info.provider === 'none') {
    return (
      <p className="text-xs text-muted-foreground" data-testid="exec-plugin-status">
        <span className="font-mono">{context}</span> uses the credentials stored in the kubeconfig.
        No exec plugin is involved.
      </p>
    )
  }
  return (
    <div className="flex flex-col gap-1 text-xs" data-testid="exec-plugin-status">
      <p>
        <span className="font-mono">{context}</span> logs in through{' '}
        {PROVIDER_LABEL[info.provider] ?? info.provider}
        {info.command && (
          <>
            {' '}
            (<code className="font-mono">{info.command}</code>)
          </>
        )}
        .{' '}
        <span data-testid="exec-plugin-installed">
          {info.installed === false
            ? 'The plugin was not found on your PATH.'
            : info.installed
              ? 'The plugin is installed.'
              : ''}
        </span>
      </p>
      {info.installed === false && info.install_hint && (
        <p className="text-muted-foreground">
          Install it with{' '}
          <code className="font-mono break-all" data-testid="exec-plugin-hint">
            {info.install_hint}
          </code>
          , then reconnect.
        </p>
      )}
      {!info.exec_policy_allowed && (
        <p className="text-muted-foreground">
          Fathom blocks this command: under the default policy only known cloud auth plugins named
          as a bare command (resolved from PATH) may run. Cluster settings can allow it for this
          context.
        </p>
      )}
    </div>
  )
}

function Section({
  title,
  detail,
  testId,
  children,
}: {
  title: string
  detail?: string
  testId: string
  children: ReactNode
}) {
  const id = useId()
  return (
    <section aria-labelledby={id} data-testid={testId} className="flex flex-col gap-2">
      <h3 id={id} className="font-display text-xs font-semibold tracking-wide uppercase">
        {title}
      </h3>
      {detail && <p className="text-xs text-muted-foreground">{detail}</p>}
      {children}
    </section>
  )
}

export function SignInSettings({
  context,
  compact = false,
}: {
  /** Limit context authentication and the exec plugin to this context (the setup step). */
  context?: string
  /** With no issuer yet, show "Add an identity provider" instead of the open form (setup). */
  compact?: boolean
}) {
  const issuers = useStore((s) => s.identity.config.issuers)
  const active = useStore((s) => s.session.activeContext)
  const [adding, setAdding] = useState(false)
  const execContext = context ?? active
  const showForm = !compact || issuers.length > 0 || adding

  return (
    <div className="flex flex-col gap-5" data-testid="sign-in-settings">
      <Section
        title="Identity providers"
        detail="Sign in with an OIDC provider your clusters trust. Contexts keep using their kubeconfig credentials unless you choose a provider for them below."
        testId="sign-in-issuers"
      >
        {showForm ? (
          <IssuerSettings />
        ) : (
          <div className="flex flex-col items-start gap-2">
            <p className="text-xs text-muted-foreground" data-testid="sign-in-no-issuer">
              No identity provider is set up. You only need one if your cluster uses OIDC.
            </p>
            <Button
              size="sm"
              variant="outline"
              data-testid="sign-in-add-issuer"
              onClick={() => setAdding(true)}
            >
              <PlusIcon data-icon="inline-start" /> Add an identity provider
            </Button>
          </div>
        )}
      </Section>
      {issuers.length > 0 && (
        <Section
          title="Context authentication"
          detail="Which credentials Fathom uses for each context."
          testId="sign-in-context-auth"
        >
          <ContextAuth {...(context !== undefined ? { only: context } : {})} />
        </Section>
      )}
      {execContext !== undefined && <ExecPluginSection context={execContext} />}
    </div>
  )
}

function ExecPluginSection({ context }: { context: string }) {
  const known = useStore((s) => s.session.cloudAuth[context] !== undefined)
  if (!known) return null
  return (
    <Section title="Exec plugin" testId="sign-in-exec-plugin">
      <ExecPluginStatus context={context} />
    </Section>
  )
}
