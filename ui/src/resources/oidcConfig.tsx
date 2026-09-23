/**
 * OIDC issuer config generator (011-rbac-oidc-administration, US5, T047-T049), the standalone
 * panel for the `access/oidc-config` section. This is a frontend-only CONFIG GENERATOR: it never
 * reads or writes cluster state beyond the best-effort `RefreshKubeadmDetection` informational
 * probe, and it never sends any mutation command. It is intentionally NOT a `ResourceDescriptor`
 * (no list/detail split) and is registered specially for its section, the same way
 * `PortForwardsPanel`/`HubblePanel` are standalone tool panels rather than resource descriptors.
 *
 * Deliberately structured and styled to look nothing like `YamlDiffView`'s diff-preview/apply
 * flow (no diff-line coloring, no Apply button, no `ConfirmDialog`) — see the panel's own
 * `rbac-oidc-generator-*` test ids and the `oidc-generator-*` container classes, none of which
 * overlap with the yaml-diff/apply naming used elsewhere in this app.
 */
import { CheckCircleIcon, CopyIcon, DownloadSimpleIcon, QuestionIcon } from '@phosphor-icons/react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useStore } from '@/store'

import {
  buildKubeadmFlags,
  buildProviderInstructions,
  type OidcFormInput,
} from './oidcGenerator'

const DEFAULT_FORM: OidcFormInput = {
  issuerUrl: '',
  clientId: '',
  usernameClaim: 'email',
  groupsClaim: 'groups',
  usernamePrefix: '',
  groupsPrefix: '',
  audience: '',
  extraScopes: '',
}

function cleanInput(form: OidcFormInput): OidcFormInput {
  return {
    issuerUrl: form.issuerUrl.trim(),
    clientId: form.clientId.trim(),
    usernameClaim: form.usernameClaim.trim() || 'sub',
    groupsClaim: form.groupsClaim.trim() || 'groups',
    usernamePrefix: form.usernamePrefix?.trim() || undefined,
    groupsPrefix: form.groupsPrefix?.trim() || undefined,
    audience: form.audience?.trim() || undefined,
    extraScopes: form.extraScopes?.trim() || undefined,
  }
}

function renderExportText(flags: string[], blocks: ReturnType<typeof buildProviderInstructions>): string {
  const lines: string[] = []
  lines.push('# Fathom OIDC issuer config (generated, not applied)')
  lines.push('#')
  lines.push('# This generates configuration for you to apply yourself outside Fathom — Fathom')
  lines.push('# does not modify the API server. Applying kube-apiserver flags typically requires')
  lines.push("# editing the control plane's static pod manifest or your managed offering's own")
  lines.push('# configuration path, and will likely restart the API server.')
  lines.push('')
  lines.push('## kube-apiserver flags (self-managed / kubeadm clusters)')
  lines.push(...flags)
  lines.push('')
  for (const block of blocks) {
    lines.push(`## ${block.provider}`)
    for (const line of block.instructions) lines.push(`- ${line}`)
    lines.push('')
  }
  return lines.join('\n')
}

export function OidcConfigView() {
  const request = useStore((s) => s.request)
  const detection = useStore((s) => s.rbacAccess.kubeadmDetection)

  useEffect(() => {
    void request({ type: 'RefreshKubeadmDetection' }).catch(() => {
      // Best-effort probe (never surfaced as an error) — generation works regardless.
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const [form, setForm] = useState<OidcFormInput>(DEFAULT_FORM)
  const [generated, setGenerated] = useState<OidcFormInput | undefined>(undefined)

  const flags = useMemo(() => (generated ? buildKubeadmFlags(generated) : []), [generated])
  const providerBlocks = useMemo(
    () => (generated ? buildProviderInstructions(generated) : []),
    [generated],
  )

  const canGenerate = form.issuerUrl.trim().length > 0 && form.clientId.trim().length > 0

  const handleGenerate = () => {
    setGenerated(cleanInput(form))
  }

  const handleCopy = () => {
    void navigator.clipboard.writeText(flags.join('\n')).then(() => {
      toast.success('Copied kube-apiserver flags', { testId: 'rbac-oidc-generator-copy-toast' })
    })
  }

  const handleExport = () => {
    if (!generated) return
    const text = renderExportText(flags, providerBlocks)
    const blob = new Blob([text], { type: 'text/plain' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = 'fathom-oidc-config.txt'
    document.body.appendChild(a)
    a.click()
    a.remove()
    URL.revokeObjectURL(url)
  }

  const detectionLabel = !detection
    ? 'Checking cluster…'
    : !detection.checked
      ? 'Not determinable'
      : detection.detected
        ? 'Kubeadm cluster detected'
        : 'Not a kubeadm cluster'

  return (
    <div
      className="oidc-generator-panel flex h-full min-h-0 flex-col gap-4 overflow-auto p-4"
      data-testid="rbac-oidc-generator-panel"
    >
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-border pb-3">
        <div>
          <h2 className="font-display text-sm font-semibold tracking-wide uppercase">
            OIDC Issuer Config Generator
          </h2>
          <p className="mt-1 max-w-prose text-xs text-muted-foreground">
            Build the kube-apiserver OIDC flags and per-provider guidance for wiring an external
            identity provider into this cluster's authentication. This is a generator only —
            nothing here is sent to the cluster.
          </p>
        </div>
        <Badge
          variant="outline"
          data-testid="rbac-oidc-generator-kubeadm-badge"
          data-detected={detection?.detected ?? false}
          data-checked={detection?.checked ?? false}
        >
          {detectionLabel}
        </Badge>
      </header>

      <div className="oidc-generator-form grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Field label="Issuer URL" testId="rbac-oidc-issuer-url">
          <Input
            value={form.issuerUrl}
            placeholder="https://accounts.example.com"
            onChange={(e) => setForm((f) => ({ ...f, issuerUrl: e.target.value }))}
          />
        </Field>
        <Field label="Client ID" testId="rbac-oidc-client-id">
          <Input
            value={form.clientId}
            placeholder="fathom-cluster"
            onChange={(e) => setForm((f) => ({ ...f, clientId: e.target.value }))}
          />
        </Field>
        <Field label="Username claim" testId="rbac-oidc-username-claim">
          <Input
            value={form.usernameClaim}
            onChange={(e) => setForm((f) => ({ ...f, usernameClaim: e.target.value }))}
          />
        </Field>
        <Field label="Groups claim" testId="rbac-oidc-groups-claim">
          <Input
            value={form.groupsClaim}
            onChange={(e) => setForm((f) => ({ ...f, groupsClaim: e.target.value }))}
          />
        </Field>
        <Field label="Username prefix (optional)" testId="rbac-oidc-username-prefix">
          <Input
            value={form.usernamePrefix}
            onChange={(e) => setForm((f) => ({ ...f, usernamePrefix: e.target.value }))}
          />
        </Field>
        <Field label="Groups prefix (optional)" testId="rbac-oidc-groups-prefix">
          <Input
            value={form.groupsPrefix}
            onChange={(e) => setForm((f) => ({ ...f, groupsPrefix: e.target.value }))}
          />
        </Field>
        <Field label="Audience (optional)" testId="rbac-oidc-audience">
          <Input
            value={form.audience}
            placeholder="https://kubernetes.example.com"
            onChange={(e) => setForm((f) => ({ ...f, audience: e.target.value }))}
          />
        </Field>
        <Field label="Extra scopes (optional, comma-separated)" testId="rbac-oidc-extra-scopes">
          <Input
            value={form.extraScopes}
            placeholder="openid, profile, email"
            onChange={(e) => setForm((f) => ({ ...f, extraScopes: e.target.value }))}
          />
        </Field>
      </div>

      <div>
        <Button
          data-testid="rbac-oidc-generate"
          disabled={!canGenerate}
          onClick={handleGenerate}
        >
          Generate
        </Button>
      </div>

      {generated && (
        <section
          className="oidc-generator-output flex min-h-0 flex-col gap-3 border border-border bg-muted/20 p-3"
          data-testid="rbac-oidc-generator-output"
        >
          <p
            className="flex items-start gap-2 border border-status-warn/50 bg-status-warn/10 px-2 py-1.5 text-xs text-status-warn"
            data-testid="rbac-oidc-generator-disclaimer"
          >
            <QuestionIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
            This generates configuration for you to apply yourself outside Fathom — Fathom does
            not modify the API server. Applying kube-apiserver flags typically requires editing
            the control plane's static pod manifest or your managed offering's own configuration
            path, and will likely restart the API server.
          </p>

          <div className="flex items-center justify-between gap-2">
            <h3 className="font-display text-xs font-semibold tracking-wide uppercase">
              kube-apiserver flags
            </h3>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="xs"
                data-testid="rbac-oidc-generator-copy"
                onClick={handleCopy}
              >
                <CopyIcon aria-hidden="true" />
                Copy
              </Button>
              <Button
                variant="outline"
                size="xs"
                data-testid="rbac-oidc-generator-export"
                onClick={handleExport}
              >
                <DownloadSimpleIcon aria-hidden="true" />
                Export
              </Button>
            </div>
          </div>
          <pre
            data-testid="rbac-oidc-generator-flags"
            className="min-w-0 overflow-auto bg-background p-2 font-mono text-xs leading-relaxed whitespace-pre-wrap"
          >
            {flags.join('\n')}
          </pre>

          <h3 className="font-display text-xs font-semibold tracking-wide uppercase">
            Managed offering guidance
          </h3>
          <div className="flex flex-col gap-3">
            {providerBlocks.map((block) => (
              <div
                key={block.provider}
                className="oidc-provider-block border border-border/60 p-2"
                data-testid={`rbac-oidc-provider-${block.provider.toLowerCase()}`}
              >
                <p className="mb-1 flex items-center gap-1.5 text-xs font-semibold">
                  <CheckCircleIcon className="size-3.5 text-accent-neon" aria-hidden="true" />
                  {block.provider}
                </p>
                <ul className="list-disc space-y-1 pl-4 text-xs text-muted-foreground">
                  {block.instructions.map((line, i) => (
                    <li key={i}>{line}</li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </section>
      )}
    </div>
  )
}

function Field({
  label,
  testId,
  children,
}: {
  label: string
  testId: string
  children: React.ReactNode
}) {
  return (
    <label className="flex flex-col gap-1 text-xs" data-testid={testId}>
      <span className="text-muted-foreground">{label}</span>
      {children}
    </label>
  )
}
