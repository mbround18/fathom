/**
 * Settings → Identity (spec 060 US5): the OIDC role mapping, read-only (it lives in the server's
 * config file), and the warnings the server raises about it. While a mapping is active, roles of
 * people who sign in through the identity provider are managed there, not in Users.
 */
import { IdentificationCardIcon, WarningIcon } from '@phosphor-icons/react'
import { Badge } from '@companion/ui'
import { useEffect } from 'react'

import { RefusalText } from '@/features/teams/parts'
import type { RoleMappingInfo } from '@/protocol/wire'
import { EmptyState } from '@/ui/EmptyState'
import { LoadState } from '@/ui/LoadState'

import { useSettings } from './store'

/** What each warning code means; an unknown code is shown as sent. */
const WARNING: Record<string, string> = {
  idp_names_no_owner:
    'The mapping names no owner group, so no one signing in through the identity provider becomes an owner. Keep a local owner.',
  role_mapping_without_oidc: 'A role mapping is configured, but OIDC sign-in is off, so it has no effect.',
}

function Groups({ label, names }: { label: string; names: string[] }) {
  return (
    <div className="contents">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 font-mono break-words">
        {names.length === 0 ? <span className="text-muted-foreground">none</span> : names.join(', ')}
      </dd>
    </div>
  )
}

function Mapping({ mapping }: { mapping: RoleMappingInfo }) {
  return (
    <section aria-label="Role mapping" className="flex min-w-0 flex-col gap-2 border border-border p-3 text-xs">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <h2 className="font-display text-base font-semibold">Role mapping</h2>
        <Badge variant="outline">read-only</Badge>
      </div>
      <dl className="grid min-w-0 grid-cols-[minmax(0,8rem)_minmax(0,1fr)] gap-x-3 gap-y-1">
        <Groups label="Provider" names={[mapping.provider]} />
        <Groups label="Claim" names={[mapping.claim]} />
        <Groups label="Owner" names={mapping.owner} />
        <Groups label="Admin" names={mapping.admin} />
        <Groups label="User" names={mapping.user} />
        <div className="contents">
          <dt className="text-muted-foreground">Unmapped sign-in</dt>
          <dd>{mapping.required ? 'Refused' : 'Signed in as a user'}</dd>
        </div>
      </dl>
      <p className="text-muted-foreground">
        Change it in the server&apos;s config file. While it is active, roles of people who sign in through the identity
        provider are set there and cannot be changed in Users.
      </p>
    </section>
  )
}

export function IdentityTab() {
  const { value, refusal } = useSettings((s) => s.settings)
  const load = useSettings((s) => s.loadSettings)
  useEffect(() => {
    load()
  }, [load])
  return (
    <LoadState loading={value === null} error={refusal && <RefusalText refusal={refusal} />} label="Loading settings…">
      {value && (
        <div className="flex min-w-0 flex-col gap-3">
          {value.warnings.length > 0 && (
            <ul aria-label="Warnings" className="flex min-w-0 flex-col gap-1">
              {value.warnings.map((w) => (
                <li
                  key={w}
                  data-warning={w}
                  role="alert"
                  className="flex min-w-0 items-start gap-1.5 border border-status-warn/60 px-2 py-1.5 text-xs text-status-warn"
                >
                  <WarningIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
                  <span className="min-w-0 break-words">{WARNING[w] ?? w}</span>
                </li>
              ))}
            </ul>
          )}
          {value.role_mapping ? (
            <Mapping mapping={value.role_mapping} />
          ) : (
            <EmptyState icon={<IdentificationCardIcon aria-hidden />} title="No role mapping">
              Roles are set in Users. To take them from your identity provider, add a role mapping to the server&apos;s
              OIDC config.
            </EmptyState>
          )}
        </div>
      )}
    </LoadState>
  )
}
