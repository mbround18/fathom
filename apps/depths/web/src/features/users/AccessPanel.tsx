/**
 * One person's sessions and networks, for an administrator (spec 052 US3). Ending their sessions
 * works from any network; whitelisting, removing and forgetting a network work only from a trusted
 * one (FR-006, FR-017), and the server refuses them otherwise. Loaded on demand from the Users page.
 */
import { Button } from '@companion/ui'
import { LoadState } from '@companion/ui/kit'
import { useCallback, useEffect, useId, useState, type SyntheticEvent } from 'react'

import { useSession } from '@/app/session-store'
import {
  endUserSessions,
  LIST_LOADING,
  listRead,
  removeNetwork,
  userNetworks,
  userSessions,
  whitelistNetwork,
  type ApiResult,
  type ListRead,
} from '@/features/network/api'
import { useUntrusted } from '@/features/network/model'
import { NetworkList, SessionList } from '@/features/sessions/lists'
import { whitelistProblem } from '@/features/sessions/model'
import type { NetworkView, SessionView } from '@/protocol/generated'

export default function AccessPanel({ id, name }: { id: string; name: string }) {
  const csrf = useSession((s) => s.me?.csrf_token ?? '')
  const refreshMe = useSession((s) => s.refreshMe)
  const blocked = useUntrusted()
  const [sessions, setSessions] = useState<ListRead<SessionView[]>>(LIST_LOADING)
  const [networks, setNetworks] = useState<ListRead<NetworkView[]>>(LIST_LOADING)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null)

  const loadSessions = useCallback(async (): Promise<void> => setSessions(listRead(await userSessions(id))), [id])
  const loadNetworks = useCallback(async (): Promise<void> => setNetworks(listRead(await userNetworks(id))), [id])

  useEffect(() => {
    let live = true
    void userSessions(id).then((r) => live && setSessions(listRead(r)))
    void userNetworks(id).then((r) => live && setNetworks(listRead(r)))
    return () => {
      live = false
    }
  }, [id])

  const run = async (what: () => Promise<ApiResult<unknown>>, done: string): Promise<boolean> => {
    setBusy(true)
    const r = await what()
    if (r.ok) setNote({ kind: 'ok', text: done })
    else if (r.error === 'untrusted_network') {
      // The network stopped being trusted mid-session: say so, and let the page catch up.
      setNote({ kind: 'err', text: 'Refused: not from this network. Whitelisting needs a trusted network.' })
      await refreshMe()
    } else setNote({ kind: 'err', text: 'That did not work. Try again.' })
    await Promise.all([loadSessions(), loadNetworks()])
    setBusy(false)
    return r.ok
  }

  const endAll = (): void => void run(() => endUserSessions(csrf, id), `Every session for ${name} ended.`)
  const remove = (n: NetworkView): void =>
    void run(
      () => removeNetwork(csrf, id, n.key),
      n.source === 'whitelist' ? `${n.key} is no longer whitelisted; it is unknown again.` : `${n.key} forgotten.`,
    )

  return (
    <div role="region" aria-label={`Sessions and networks for ${name}`} data-panel="access" className="flex min-w-0 flex-col gap-2 border-t border-border pt-1.5">
      {note && (
        <p role={note.kind === 'err' ? 'alert' : 'status'} className={`text-xs ${note.kind === 'err' ? 'text-status-err' : 'text-muted-foreground'}`}>
          {note.text}
        </p>
      )}
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="font-semibold">Sessions</span>
        <span className="flex-1" />
        <Button variant="outline" size="xs" disabled={busy || (sessions.value?.length ?? 0) === 0} onClick={endAll} aria-label={`End every session for ${name}`}>
          End every session
        </Button>
      </div>
      <LoadState loading={sessions.value === null} failed={sessions.failed} what={`sessions for ${name}`} onRetry={() => void loadSessions()}>
        <SessionList sessions={sessions.value ?? []} label={`Sessions for ${name}`} />
      </LoadState>

      <span className="font-semibold">Networks</span>
      <LoadState loading={networks.value === null} failed={networks.failed} what={`networks for ${name}`} onRetry={() => void loadNetworks()}>
        <NetworkList networks={networks.value ?? []} label={`Networks for ${name}`} onRemove={remove} removeBlocked={blocked} busy={busy} />
      </LoadState>
      <WhitelistForm
        name={name}
        blocked={blocked}
        busy={busy}
        onAdd={(cidr, label) => run(() => whitelistNetwork(csrf, id, label === '' ? { cidr } : { cidr, label }), `${cidr} whitelisted for ${name}.`)}
      />
    </div>
  )
}

function WhitelistForm({
  name,
  blocked,
  busy,
  onAdd,
}: {
  name: string
  blocked: string | null
  busy: boolean
  onAdd: (cidr: string, label: string) => Promise<boolean>
}) {
  const [cidr, setCidr] = useState('')
  const [label, setLabel] = useState('')
  const [problem, setProblem] = useState<string | null>(null)
  const hint = useId()

  const submit = (e: SyntheticEvent): void => {
    e.preventDefault()
    const v = cidr.trim()
    const p = whitelistProblem(v)
    setProblem(p)
    if (p !== null) return
    void onAdd(v, label.trim()).then((ok) => {
      if (ok) {
        setCidr('')
        setLabel('')
      }
    })
  }

  return (
    <form onSubmit={submit} aria-label={`Whitelist a network for ${name}`} className="flex min-w-0 flex-wrap items-center gap-1">
      <input
        value={cidr}
        onChange={(e) => setCidr(e.target.value)}
        aria-label="Network address"
        aria-describedby={hint}
        aria-invalid={problem !== null || undefined}
        placeholder="198.51.100.0/24"
        className="h-6 min-w-0 flex-1 border border-input bg-background px-1 font-mono text-xs text-foreground"
      />
      <input
        value={label}
        onChange={(e) => setLabel(e.target.value)}
        aria-label="Label"
        placeholder="Label (optional)"
        maxLength={64}
        className="h-6 min-w-0 flex-1 border border-input bg-background px-1 text-xs text-foreground"
      />
      <Button
        type="submit"
        variant="outline"
        size="xs"
        disabled={busy || blocked !== null}
        title={blocked ?? undefined}
        data-blocked={blocked === null ? undefined : 'untrusted_network'}
      >
        Whitelist
      </Button>
      <p id={hint} role={problem === null ? undefined : 'alert'} className={`w-full ${problem === null ? 'text-muted-foreground' : 'text-status-err'}`}>
        {problem ?? 'Trusted at once for this person. Only from a trusted network.'}
      </p>
    </form>
  )
}
