/**
 * Receivers (spec 055 US1, T052): webhook endpoints, the bindings each may sync, and the
 * deliveries they recorded with their decisions. A receiver's secret is shown once, right after
 * it is created or rotated, and never again.
 */
import { KeyIcon, LinkBreakIcon } from '@phosphor-icons/react'
import { Button, Input } from '@companion/ui'
import { useId, useState, type SyntheticEvent } from 'react'

import { ErrorLine, Field, PreviewDialog, WorkspacePicker } from '@/features/bindings/parts'
import { short, useRepos } from '@/features/bindings/store'
import type { DeliveryInfo, ReceiverInfo, ReceiverProvider } from '@/protocol/wire'
import { CopyBlock } from '@/ui/CopyBlock'
import { DataTable } from '@/ui/DataTable'
import { EmptyState } from '@/ui/EmptyState'
import { FIELD, H2, LABEL, STATUS } from '@/ui/form'
import { Page } from '@/ui/Page'
import { PageHeader } from '@/ui/PageHeader'

const PROVIDERS: { provider: ReceiverProvider; label: string }[] = [
  { provider: 'github', label: 'GitHub' },
  { provider: 'gitlab', label: 'GitLab' },
  { provider: 'forgejo', label: 'Forgejo / Gitea' },
  { provider: 'generic', label: 'Generic (HMAC)' },
]

function Secret() {
  const secret = useRepos((s) => s.secret)
  const dismiss = useRepos((s) => s.dismissSecret)
  if (!secret) return null
  return (
    <section aria-label="Receiver secret" className="flex min-w-0 flex-col gap-3 border border-accent-neon bg-accent-neon/5 p-3 text-xs">
      <p className="flex items-center gap-1.5 font-semibold">
        <KeyIcon aria-hidden className="size-4 text-accent-neon" />
        Copy the secret now. Keel will not show it again.
      </p>
      <CopyBlock label="Payload URL" value={secret.url} valueData={{ 'data-hook-url': true }} />
      <CopyBlock label="Secret" value={secret.secret} valueData={{ 'data-hook-secret': true }} />
      <div>
        <Button size="sm" onClick={dismiss}>
          I have copied it
        </Button>
      </div>
    </section>
  )
}

function NewReceiver({ workspace }: { workspace: string }) {
  const propose = useRepos((s) => s.propose)
  const [name, setName] = useState('')
  const [provider, setProvider] = useState<ReceiverProvider>('github')
  const id = useId()
  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    propose({ type: 'PreviewReceiverCreate', data: { workspace, name: name.trim(), provider } })
  }
  return (
    <form
      aria-label="New receiver"
      onSubmit={submit}
      className="flex min-w-0 flex-wrap items-end gap-2 border border-border bg-muted/20 p-3"
    >
      <Field label="Name" id={`${id}-name`}>
        <Input id={`${id}-name`} value={name} onChange={(e) => setName(e.target.value)} required />
      </Field>
      <Field label="Provider" id={`${id}-provider`}>
        <select
          id={`${id}-provider`}
          className={FIELD}
          value={provider}
          onChange={(e) => setProvider(e.target.value as ReceiverProvider)}
        >
          {PROVIDERS.map((p) => (
            <option key={p.provider} value={p.provider}>
              {p.label}
            </option>
          ))}
        </select>
      </Field>
      <Button type="submit" size="sm">
        Preview receiver
      </Button>
    </form>
  )
}

function ReceiverRow({ receiver }: { receiver: ReceiverInfo }) {
  const bindings = useRepos((s) => s.bindings)
  const propose = useRepos((s) => s.propose)
  const [binding, setBinding] = useState('')
  const id = useId()
  const r = receiver.id
  const unbound = bindings.filter((b) => !receiver.bindings.includes(b.id))
  const urlOf = (b: string) => bindings.find((x) => x.id === b)?.url ?? b

  return (
    <li aria-label={`Receiver ${receiver.name}`} className="flex min-w-0 flex-col border border-border text-xs">
      <div className="flex min-h-10 min-w-0 flex-wrap items-center gap-2 border-b border-border bg-muted/20 px-3 py-1.5">
        <span className="font-mono font-semibold break-all">{receiver.name}</span>
        <span className="text-muted-foreground">{PROVIDERS.find((p) => p.provider === receiver.provider)?.label ?? receiver.provider}</span>
        <span
          data-receiver-status
          className={`${STATUS} ${receiver.status === 'active' ? 'text-status-ok' : 'text-muted-foreground'}`}
        >
          {receiver.status}
        </span>
        <div className="ml-auto flex flex-wrap gap-1">
          <Button
            size="xs"
            variant="outline"
            onClick={() => propose({ type: 'PreviewReceiverRotate', data: { receiver: r, previous_valid_for: null } })}
          >
            Rotate secret
          </Button>
          {receiver.status === 'active' && (
            <Button size="xs" variant="outline" onClick={() => propose({ type: 'PreviewReceiverDisable', data: { receiver: r } })}>
              Disable
            </Button>
          )}
          <Button size="xs" variant="destructive" onClick={() => propose({ type: 'PreviewReceiverRemove', data: { receiver: r } })}>
            Remove
          </Button>
        </div>
      </div>
      <div className="flex min-w-0 flex-col gap-3 p-3">
        <p className="font-mono break-all text-muted-foreground">{receiver.url}</p>
        {receiver.previous_expires_at && (
          <p className="text-muted-foreground">The previous secret still verifies until {receiver.previous_expires_at}.</p>
        )}
        <div className="flex min-w-0 flex-col gap-1">
          <h3 className={LABEL}>Syncs</h3>
          {receiver.bindings.length === 0 && (
            <p className="text-muted-foreground">No binding yet: deliveries are recorded but sync nothing.</p>
          )}
          <ul aria-label="Bound bindings" className="flex min-w-0 flex-col">
            {receiver.bindings.map((b) => (
              <li key={b} className="flex min-w-0 items-center gap-2 border-t border-border py-1 first:border-t-0">
                <span className="min-w-0 flex-1 font-mono break-all">{urlOf(b)}</span>
                <Button
                  size="xs"
                  variant="ghost"
                  onClick={() => propose({ type: 'PreviewReceiverUnbind', data: { receiver: r, binding: b } })}
                >
                  <LinkBreakIcon aria-hidden />
                  Unbind
                </Button>
              </li>
            ))}
          </ul>
        </div>
        {unbound.length > 0 && (
          <div className="flex min-w-0 flex-wrap items-end gap-2">
            <Field label="Bind to" id={`${id}-bind`}>
              <select id={`${id}-bind`} className={FIELD} value={binding} onChange={(e) => setBinding(e.target.value)}>
                <option value="">Choose a binding…</option>
                {unbound.map((b) => (
                  <option key={b.id} value={b.id}>
                    {b.url} @ {b.branch}
                  </option>
                ))}
              </select>
            </Field>
            <Button
              size="sm"
              variant="outline"
              disabled={!binding}
              onClick={() => propose({ type: 'PreviewReceiverBind', data: { receiver: r, binding } })}
            >
              Bind
            </Button>
          </div>
        )}
      </div>
    </li>
  )
}

function Deliveries() {
  const deliveries = useRepos((s) => s.deliveries)
  const receivers = useRepos((s) => s.receivers)
  const nameOf = (r: string) => receivers.find((x) => x.id === r)?.name ?? r
  return (
    <DataTable<DeliveryInfo>
      label="Deliveries"
      rows={deliveries}
      rowKey={(d) => d.id}
      rowData={(d) => ({ 'data-decision': d.decision })}
      columns={[
        { key: 'at', header: 'Received', cell: (d) => <span className="tabular-nums">{d.received_at}</span> },
        { key: 'receiver', header: 'Receiver', cell: (d) => nameOf(d.receiver) },
        { key: 'delivery', header: 'Delivery', cell: (d) => <span className="font-mono break-all">{d.provider_delivery_id}</span> },
        {
          key: 'decision',
          header: 'Decision',
          cell: (d) => (
            <span className={d.decision === 'refused' ? 'text-status-err' : 'text-status-ok'}>
              {d.decision}
              {d.refusal ? `: ${d.refusal}` : ''}
            </span>
          ),
        },
        { key: 'sync', header: 'Sync', cell: (d) => d.sync_state ?? <span className="text-muted-foreground">none yet</span> },
        { key: 'commit', header: 'Commit', cell: (d) => <span className="font-mono">{short(d.commit)}</span> },
      ]}
      empty="No deliveries yet. They appear here live as your Git host sends them."
    />
  )
}

export default function ReceiversPage() {
  const workspace = useRepos((s) => s.workspace)
  const receivers = useRepos((s) => s.receivers)
  return (
    <Page name="receivers" labelledBy="receivers-title">
      <PageHeader
        id="receivers-title"
        title="Receivers"
        description="Webhook endpoints your Git host calls, the bindings each one syncs, and what arrived."
      >
        <WorkspacePicker />
      </PageHeader>
      <ErrorLine />
      <Secret />
      {workspace && (
        <>
          <NewReceiver workspace={workspace} />
          {receivers.length === 0 ? (
            <EmptyState title="No receivers yet">
              Create one, then add its URL and secret at your Git host.
            </EmptyState>
          ) : (
            <ul aria-label="Receivers" className="flex min-w-0 flex-col gap-2">
              {receivers.map((r) => (
                <ReceiverRow key={r.id} receiver={r} />
              ))}
            </ul>
          )}
          <section aria-labelledby="deliveries-title" className="flex min-w-0 flex-col gap-2">
            <h2 id="deliveries-title" className={H2}>
              Deliveries
            </h2>
            <Deliveries />
          </section>
        </>
      )}
      <PreviewDialog />
    </Page>
  )
}
