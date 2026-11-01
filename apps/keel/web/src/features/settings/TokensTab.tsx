/**
 * Settings → Tokens (spec 060 US3): every enrolment token and every API token on the instance.
 *
 * Enrolment tokens can be filtered by state. A pending one can be revoked; any one not yet used
 * can be rolled, which mints a replacement and shows it in the Enroll dialog's result step. API
 * tokens can be filtered by state and an active one revoked. Every change goes through the
 * confirm dialog; the lists are read again after it.
 */
import { ArrowsClockwiseIcon, ProhibitIcon } from '@phosphor-icons/react'
import { Badge, Button } from '@companion/ui'
import { useEffect, useId } from 'react'

import { useEnroll } from '@/features/hosts/enroll-store'
import { NativeSelect } from '@/features/teams/parts'
import type { AdminApiTokenInfo, ApiTokenState, EnrollmentInfo, EnrollmentState } from '@/protocol/wire'
import { DataTable, type Column } from '@/ui/DataTable'
import { LoadState } from '@/ui/LoadState'

import { useSettings } from './store'

const H2 = 'font-display text-base font-semibold'

const ENROLLMENT_STATES: readonly EnrollmentState[] = ['pending', 'used', 'revoked', 'burned', 'expired']
const API_TOKEN_STATES: readonly ApiTokenState[] = ['active', 'expired', 'revoked']

/** RFC 3339 to minutes, UTC: `2026-10-28 14:05`. The full value is the title. */
function when(rfc3339: string): string {
  return rfc3339.slice(0, 16).replace('T', ' ')
}

function Time({ at }: { at: string }) {
  return (
    <time dateTime={at} title={at} className="text-muted-foreground tabular-nums">
      {when(at)}
    </time>
  )
}

const STATE_VARIANT: Record<string, 'default' | 'outline' | 'secondary' | 'destructive'> = {
  pending: 'default',
  active: 'default',
  used: 'secondary',
  burned: 'destructive',
}

function State({ state }: { state: string }) {
  return (
    <Badge variant={STATE_VARIANT[state] ?? 'outline'} data-state={state}>
      {state}
    </Badge>
  )
}

function Filter<S extends string>({
  label,
  states,
  value,
  onChange,
}: {
  label: string
  states: readonly S[]
  value: S | null
  onChange: (state: S | null) => void
}) {
  const id = useId()
  return (
    <>
      <label htmlFor={id} className="text-xs text-muted-foreground">
        {label}
      </label>
      <NativeSelect
        id={id}
        value={value ?? ''}
        onChange={(e) => {
          const picked = states.find((s) => s === e.target.value)
          onChange(picked ?? null)
        }}
      >
        <option value="">All</option>
        {states.map((s) => (
          <option key={s} value={s}>
            {s}
          </option>
        ))}
      </NativeSelect>
    </>
  )
}

const ENROLLMENT_COLUMNS: readonly Column<EnrollmentInfo>[] = [
  {
    key: 'name',
    header: 'Host',
    cell: (e) => (
      <span className="font-mono">
        {e.inventory_name}
        {e.rolled_from && <span className="text-muted-foreground"> (rolled)</span>}
      </span>
    ),
  },
  { key: 'workspace', header: 'Workspace', cell: (e) => e.workspace_slug },
  {
    key: 'profiles',
    header: 'Profiles',
    cell: (e) => <span className="font-mono">{e.profiles.join(', ')}</span>,
  },
  { key: 'state', header: 'State', cell: (e) => <State state={e.state} />, className: 'w-24' },
  { key: 'created', header: 'Created', cell: (e) => <Time at={e.created_at} /> },
  { key: 'expires', header: 'Expires', cell: (e) => <Time at={e.expires_at} /> },
]

function Enrollments() {
  const { value, refusal } = useSettings((s) => s.enrollments)
  const filter = useSettings((s) => s.enrollmentFilter)
  const load = useSettings((s) => s.loadEnrollments)
  const revoke = useSettings((s) => s.revokeEnrollment)
  const roll = useEnroll((s) => s.roll)
  const minted = useEnroll((s) => s.mintedCount)
  useEffect(() => {
    load()
  }, [load, minted])

  return (
    <section aria-labelledby="enrollment-tokens-h" data-pane="enrollment-tokens" className="flex min-w-0 flex-col gap-2">
      <h2 id="enrollment-tokens-h" className={H2}>
        Enrolment tokens
      </h2>
      <LoadState loading={value === null && !refusal} failed={refusal}
        what="enrolment tokens"
        onRetry={() => {
          load()
        }}
        label="Loading tokens…">
        <DataTable
          label="Enrolment tokens"
          columns={ENROLLMENT_COLUMNS}
          rows={value ?? []}
          rowKey={(e) => e.id}
          rowData={(e) => ({ 'data-enrollment': e.id, 'data-enrollment-state': e.state })}
          filter={<Filter label="State" states={ENROLLMENT_STATES} value={filter} onChange={load} />}
          empty={filter ? `No ${filter} enrolment tokens.` : 'No enrolment tokens yet. Enroll a host from Hosts.'}
          actions={(e) => (
            <>
              {e.state !== 'used' && (
                <Button
                  variant="outline"
                  size="xs"
                  aria-label={`Roll the token for ${e.inventory_name}`}
                  onClick={() => {
                    roll(e.id)
                  }}
                >
                  <ArrowsClockwiseIcon aria-hidden />
                  Roll
                </Button>
              )}
              {e.state === 'pending' && (
                <Button
                  variant="outline"
                  size="xs"
                  aria-label={`Revoke the token for ${e.inventory_name}`}
                  onClick={() => {
                    revoke(e)
                  }}
                >
                  <ProhibitIcon aria-hidden />
                  Revoke
                </Button>
              )}
            </>
          )}
        />
      </LoadState>
    </section>
  )
}

const API_TOKEN_COLUMNS: readonly Column<AdminApiTokenInfo>[] = [
  { key: 'name', header: 'Name', cell: (t) => <span className="font-mono">{t.token.name}</span> },
  { key: 'holder', header: 'Holder', cell: (t) => t.holder_name },
  { key: 'workspace', header: 'Workspace', cell: (t) => t.workspace_slug },
  {
    key: 'actions',
    header: 'Actions',
    cell: (t) => <span className="font-mono">{t.token.actions.join(', ')}</span>,
  },
  { key: 'state', header: 'State', cell: (t) => <State state={t.state} />, className: 'w-24' },
  {
    key: 'used',
    header: 'Last used',
    cell: (t) => (t.token.last_used_at ? <Time at={t.token.last_used_at} /> : <span className="text-muted-foreground">never</span>),
  },
  { key: 'expires', header: 'Expires', cell: (t) => <Time at={t.token.expires_at} /> },
]

function ApiTokens() {
  const { value, refusal } = useSettings((s) => s.apiTokens)
  const filter = useSettings((s) => s.apiTokenFilter)
  const load = useSettings((s) => s.loadApiTokens)
  const revoke = useSettings((s) => s.revokeApiToken)
  useEffect(() => {
    load()
  }, [load])

  return (
    <section aria-labelledby="api-tokens-h" data-pane="api-tokens" className="flex min-w-0 flex-col gap-2">
      <h2 id="api-tokens-h" className={H2}>
        API tokens
      </h2>
      <LoadState loading={value === null && !refusal} failed={refusal}
        what="API tokens"
        onRetry={() => {
          load()
        }}
        label="Loading tokens…">
        <DataTable
          label="API tokens"
          columns={API_TOKEN_COLUMNS}
          rows={value ?? []}
          rowKey={(t) => t.token.id}
          rowData={(t) => ({ 'data-api-token': t.token.id, 'data-api-token-state': t.state })}
          filter={<Filter label="State" states={API_TOKEN_STATES} value={filter} onChange={load} />}
          empty={filter ? `No ${filter} API tokens.` : 'No API tokens. People create their own in Users.'}
          actions={(t) =>
            t.state === 'active' && (
              <Button
                variant="outline"
                size="xs"
                aria-label={`Revoke API token ${t.token.name}`}
                onClick={() => {
                  revoke(t)
                }}
              >
                <ProhibitIcon aria-hidden />
                Revoke
              </Button>
            )
          }
        />
      </LoadState>
    </section>
  )
}

export function TokensTab() {
  return (
    <div className="flex min-w-0 flex-col gap-6">
      <Enrollments />
      <ApiTokens />
    </div>
  )
}
