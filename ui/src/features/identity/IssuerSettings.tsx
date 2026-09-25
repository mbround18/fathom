/** 033 US1: add/edit/remove OIDC issuers (public client, no secrets). */
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { OidcIssuerConfig } from '@/protocol/types'
import { useStore } from '@/store'

import { RoleMapEditor } from './RoleMapEditor'
import { SignInPanel } from './SignInPanel'

const EMPTY_ISSUER: OidcIssuerConfig = {
  id: '',
  issuer_url: '',
  client_id: '',
  scopes: ['openid', 'profile', 'email', 'groups'],
  ca_bundle_path: null,
  username_claim: 'preferred_username',
  groups_claim: 'groups',
  username_prefix: 'oidc:',
  groups_prefix: 'oidc:',
  admin_api_base: null,
}

function Field({
  label,
  testId,
  value,
  onChange,
  placeholder,
}: {
  label: string
  testId: string
  value: string
  onChange: (v: string) => void
  placeholder?: string
}) {
  return (
    <label className="flex flex-col gap-1 text-xs" data-testid={testId}>
      <span className="text-muted-foreground">{label}</span>
      <Input value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
    </label>
  )
}

export function IssuerSettings() {
  const send = useStore((s) => s.send)
  const issuers = useStore((s) => s.identity.config.issuers)
  const configError = useStore((s) => s.identity.errors[''])
  const [form, setForm] = useState<OidcIssuerConfig>(EMPTY_ISSUER)
  const set = (patch: Partial<OidcIssuerConfig>) => setForm((f) => ({ ...f, ...patch }))
  const canSave = form.id.trim() && form.issuer_url.trim() && form.client_id.trim()

  return (
    <div className="flex flex-col gap-4" data-testid="oidc-issuer-settings">
      {issuers.length === 0 && (
        <p className="text-xs text-muted-foreground" data-testid="oidc-no-issuers">
          No OIDC issuers configured.
        </p>
      )}
      {issuers.map((issuer) => (
        <section
          key={issuer.id}
          className="border border-border p-2"
          data-testid={`oidc-issuer-${issuer.id}`}
        >
          <div className="mb-2 flex items-center justify-between gap-2 text-xs">
            <span className="font-semibold">{issuer.id}</span>
            <span className="min-w-0 truncate font-mono text-muted-foreground">
              {issuer.issuer_url}
            </span>
            <Button
              size="xs"
              variant="ghost"
              data-testid={`oidc-edit-${issuer.id}`}
              onClick={() => setForm(issuer)}
            >
              Edit
            </Button>
            <Button
              size="xs"
              variant="ghost"
              data-testid={`oidc-delete-${issuer.id}`}
              onClick={() => send({ type: 'OidcDeleteIssuer', data: { issuer_id: issuer.id } })}
            >
              Remove
            </Button>
          </div>
          <SignInPanel issuer={issuer} />
        </section>
      ))}

      <RoleMapEditor />

      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <Field
          label="Issuer id"
          testId="oidc-form-id"
          value={form.id}
          onChange={(id) => set({ id })}
        />
        <Field
          label="Issuer URL"
          testId="oidc-form-issuer-url"
          value={form.issuer_url}
          placeholder="https://auth.example.com/auth/v1"
          onChange={(issuer_url) => set({ issuer_url })}
        />
        <Field
          label="Client ID (public client)"
          testId="oidc-form-client-id"
          value={form.client_id}
          onChange={(client_id) => set({ client_id })}
        />
        <Field
          label="Private CA bundle path (optional)"
          testId="oidc-form-ca"
          value={form.ca_bundle_path ?? ''}
          onChange={(v) => set({ ca_bundle_path: v.trim() || null })}
        />
        <Field
          label="Username claim"
          testId="oidc-form-username-claim"
          value={form.username_claim}
          onChange={(username_claim) => set({ username_claim })}
        />
        <Field
          label="Groups claim"
          testId="oidc-form-groups-claim"
          value={form.groups_claim}
          onChange={(groups_claim) => set({ groups_claim })}
        />
      </div>
      {configError && (
        <p role="alert" className="text-status-error text-xs" data-testid="oidc-config-error">
          {configError.message}
        </p>
      )}
      <div>
        <Button
          size="sm"
          disabled={!canSave}
          data-testid="oidc-form-save"
          onClick={() => {
            send({
              type: 'OidcSaveIssuer',
              data: {
                issuer: {
                  ...form,
                  id: form.id.trim(),
                  issuer_url: form.issuer_url.trim(),
                  client_id: form.client_id.trim(),
                },
              },
            })
            setForm(EMPTY_ISSUER)
          }}
        >
          Save issuer
        </Button>
      </div>
    </div>
  )
}
