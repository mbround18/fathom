/**
 * Your sessions (spec 052 US2): every live session and known network for the signed-in person.
 * Ending sessions and changing an answer work from any network (FR-015, FR-016); a change to
 * "home" starts the maturity clock, it never trusts the network at once.
 */
import { Button } from '@companion/ui'
import { useCallback, useEffect, useState } from 'react'

import { useSession } from '@/app/session-store'
import { answerNetwork, endOtherSessions, endSession, listNetworks, listSessions } from '@/features/network/api'
import type { NetworkAnswer, NetworkView, SessionView } from '@/protocol/generated'

import { NetworkList, SessionList } from './lists'

type Loaded = [Awaited<ReturnType<typeof listSessions>>, Awaited<ReturnType<typeof listNetworks>>]
const fetchAll = (): Promise<Loaded> => Promise.all([listSessions(), listNetworks()])

export default function SessionsPage() {
  const csrf = useSession((s) => s.me?.csrf_token ?? '')
  const refreshMe = useSession((s) => s.refreshMe)
  const [sessions, setSessions] = useState<SessionView[] | null>(null)
  const [networks, setNetworks] = useState<NetworkView[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null)

  const show = useCallback(([s, n]: Loaded): void => {
    if (s.ok) setSessions(s.data)
    if (n.ok) setNetworks(n.data)
    if (!s.ok || !n.ok) setNote({ kind: 'err', text: 'Could not load everything. Try again.' })
  }, [])
  const load = async (): Promise<void> => show(await fetchAll())

  useEffect(() => {
    void fetchAll().then(show)
  }, [show])

  const run = async (what: () => Promise<{ ok: boolean }>, done: string): Promise<void> => {
    setBusy(true)
    const r = await what()
    setNote(r.ok ? { kind: 'ok', text: done } : { kind: 'err', text: 'That did not work. Try again.' })
    await load()
    setBusy(false)
  }

  const endOne = (hint: string): void => void run(() => endSession(csrf, hint), 'Session ended.')
  const endOthers = (): void => void run(() => endOtherSessions(csrf), 'Every other session ended.')
  const answer = (key: string, a: NetworkAnswer): void =>
    void run(async () => {
      const r = await answerNetwork(csrf, key, a)
      await refreshMe()
      return r
    }, a === 'home' ? 'Saved: home. It is trusted once the waiting period ends and you have used it on two days.' : 'Saved: public.')

  const others = sessions?.filter((s) => !s.current).length ?? 0

  return (
    <section data-page="sessions" aria-labelledby="sessions-title" className="flex min-w-0 flex-col gap-3 p-2">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <h1 id="sessions-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Your sessions
        </h1>
        <span className="flex-1" />
        <Button variant="outline" size="sm" disabled={busy || others === 0} onClick={endOthers} data-action="end-others">
          End all other sessions
        </Button>
      </div>
      {note && (
        <p role={note.kind === 'err' ? 'alert' : 'status'} className={`text-xs ${note.kind === 'err' ? 'text-status-err' : 'text-muted-foreground'}`}>
          {note.text}
        </p>
      )}
      <section aria-labelledby="sessions-live" className="flex min-w-0 flex-col gap-1">
        <h2 id="sessions-live" className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          Signed in
        </h2>
        {sessions === null ? (
          <p className="text-xs text-muted-foreground">Loading…</p>
        ) : (
          <SessionList sessions={sessions} onEnd={endOne} busy={busy} label="Your sessions" />
        )}
      </section>
      <section aria-labelledby="sessions-networks" className="flex min-w-0 flex-col gap-1">
        <h2 id="sessions-networks" className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          Known networks
        </h2>
        <p className="text-xs text-muted-foreground">
          Locations are approximate and only shown to you; they never decide trust.
        </p>
        {networks === null ? (
          <p className="text-xs text-muted-foreground">Loading…</p>
        ) : (
          <NetworkList networks={networks} onAnswer={answer} busy={busy} label="Your networks" />
        )}
      </section>
    </section>
  )
}
