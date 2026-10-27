/**
 * Receivers (spec 055 US1, T052): webhook endpoints, the bindings each may sync, and the
 * deliveries they recorded with their decisions. A receiver's secret is shown once, right after
 * it is created or rotated, and never again.
 */
import { Button, Input } from '@companion/ui'
import { useId, useState, type SyntheticEvent } from 'react'

import { ErrorLine, Field, PreviewDialog, WorkspacePicker } from '@/features/bindings/parts'
import { FIELD, short, useRepos } from '@/features/bindings/store'
import type { ReceiverInfo, ReceiverProvider } from '@/protocol/wire'

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
    <section aria-label="Receiver secret" className="flex flex-col gap-2 border border-accent-neon p-3 text-xs">
      <p className="font-semibold">Copy the secret now. Keel will not show it again.</p>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
        <dt className="text-muted-foreground">Payload URL</dt>
        <dd data-hook-url className="font-mono break-all">
          {secret.url}
        </dd>
        <dt className="text-muted-foreground">Secret</dt>
        <dd data-hook-secret className="font-mono break-all">
          {secret.secret}
        </dd>
      </dl>
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
    <form aria-label="New receiver" onSubmit={submit} className="flex flex-wrap items-end gap-2">
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
    <li aria-label={`Receiver ${receiver.name}`} className="flex flex-col gap-2 border border-border p-3 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono font-semibold">{receiver.name}</span>
        <span className="text-muted-foreground">{receiver.provider}</span>
        <span data-receiver-status className="uppercase">
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
      <p className="font-mono break-all text-muted-foreground">{receiver.url}</p>
      {receiver.previous_expires_at && (
        <p className="text-muted-foreground">The previous secret still verifies until {receiver.previous_expires_at}.</p>
      )}
      <ul aria-label="Bound bindings" className="flex flex-col gap-1">
        {receiver.bindings.map((b) => (
          <li key={b} className="flex items-center gap-2">
            <span className="font-mono break-all">{urlOf(b)}</span>
            <Button
              size="xs"
              variant="ghost"
              onClick={() => propose({ type: 'PreviewReceiverUnbind', data: { receiver: r, binding: b } })}
            >
              Unbind
            </Button>
          </li>
        ))}
      </ul>
      {unbound.length > 0 && (
        <div className="flex flex-wrap items-end gap-2">
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
    </li>
  )
}

function Deliveries() {
  const deliveries = useRepos((s) => s.deliveries)
  const receivers = useRepos((s) => s.receivers)
  const nameOf = (r: string) => receivers.find((x) => x.id === r)?.name ?? r
  return (
    <table aria-label="Deliveries" className="w-full text-left text-xs">
      <thead className="text-muted-foreground">
        <tr>
          <th className="py-1">Received</th>
          <th>Receiver</th>
          <th>Delivery</th>
          <th>Decision</th>
          <th>Sync</th>
          <th>Commit</th>
        </tr>
      </thead>
      <tbody>
        {deliveries.length === 0 && (
          <tr>
            <td colSpan={6} data-pane-state="empty" className="py-1 text-muted-foreground">
              No deliveries yet.
            </td>
          </tr>
        )}
        {deliveries.map((d) => (
          <tr key={d.id} data-decision={d.decision} className="border-t border-border">
            <td className="py-1">{d.received_at}</td>
            <td>{nameOf(d.receiver)}</td>
            <td className="font-mono">{d.provider_delivery_id}</td>
            <td className={d.decision === 'refused' ? 'text-status-err' : ''}>
              {d.decision}
              {d.refusal ? `: ${d.refusal}` : ''}
            </td>
            <td>{d.sync_state ?? ''}</td>
            <td className="font-mono">{short(d.commit)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

export default function ReceiversPage() {
  const workspace = useRepos((s) => s.workspace)
  const receivers = useRepos((s) => s.receivers)
  return (
    <section data-page="receivers" aria-labelledby="receivers-title" className="flex flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 id="receivers-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Receivers
        </h1>
        <WorkspacePicker />
      </div>
      <ErrorLine />
      <Secret />
      {workspace && (
        <>
          <NewReceiver workspace={workspace} />
          {receivers.length === 0 ? (
            <p data-pane-state="empty" className="text-xs text-muted-foreground">
              No receivers yet. Create one, then add its URL and secret at your Git host.
            </p>
          ) : (
            <ul aria-label="Receivers" className="flex flex-col gap-2">
              {receivers.map((r) => (
                <ReceiverRow key={r.id} receiver={r} />
              ))}
            </ul>
          )}
          <h2 className="font-display text-xs font-semibold tracking-wider uppercase">Deliveries</h2>
          <Deliveries />
        </>
      )}
      <PreviewDialog />
    </section>
  )
}
