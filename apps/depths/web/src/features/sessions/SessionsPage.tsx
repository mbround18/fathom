/**
 * Your sessions (spec 052 US2): every live session and known network for the signed-in person.
 * Ending sessions and changing an answer work from any network (FR-015, FR-016); a change to
 * "home" starts the maturity clock, it never trusts the network at once.
 */
import { Button } from '@companion/ui'
import { LoadState, Page, PageHeader } from '@companion/ui/kit'
import { lazy, Suspense, useCallback, useEffect, useState } from 'react'

import { useSession } from '@/app/session-store'
import {
  answerNetwork,
  endOtherSessions,
  endSession,
  listNetworks,
  listSessions,
  listRead,
  LIST_LOADING,
  type ListRead,
} from '@/features/network/api'
import type { NetworkAnswer, NetworkView, SessionView } from '@/protocol/generated'

import { NetworkList, SessionList } from './lists'

const PasskeysPanel = lazy(() => import('@/features/passkeys/PasskeysPanel'))

export default function SessionsPage() {
  const csrf = useSession((s) => s.me?.csrf_token ?? '')
  const refreshMe = useSession((s) => s.refreshMe)
  const [sessions, setSessions] = useState<ListRead<SessionView[]>>(LIST_LOADING)
  const [networks, setNetworks] = useState<ListRead<NetworkView[]>>(LIST_LOADING)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null)

  const loadSessions = useCallback(async (): Promise<void> => setSessions(listRead(await listSessions())), [])
  const loadNetworks = useCallback(async (): Promise<void> => setNetworks(listRead(await listNetworks())), [])
  const load = async (): Promise<void> => {
    await Promise.all([loadSessions(), loadNetworks()])
  }

  useEffect(() => {
    let live = true
    void listSessions().then((r) => live && setSessions(listRead(r)))
    void listNetworks().then((r) => live && setNetworks(listRead(r)))
    return () => {
      live = false
    }
  }, [])

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

  const others = sessions.value?.filter((s) => !s.current).length ?? 0

  return (
    <Page name="sessions" labelledBy="sessions-title">
      <PageHeader
        id="sessions-title"
        title="Your sessions"
        actions={
          <Button variant="outline" size="sm" disabled={busy || others === 0} onClick={endOthers} data-action="end-others">
            End all other sessions
          </Button>
        }
      />
      {note && (
        <p role={note.kind === 'err' ? 'alert' : 'status'} className={`text-xs ${note.kind === 'err' ? 'text-status-err' : 'text-muted-foreground'}`}>
          {note.text}
        </p>
      )}
      <section aria-labelledby="sessions-live" className="flex min-w-0 flex-col gap-1">
        <h2 id="sessions-live" className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          Signed in
        </h2>
        <LoadState loading={sessions.value === null} failed={sessions.failed} what="your sessions" onRetry={() => void loadSessions()}>
          <SessionList sessions={sessions.value ?? []} onEnd={endOne} busy={busy} label="Your sessions" />
        </LoadState>
      </section>
      <section aria-labelledby="sessions-networks" className="flex min-w-0 flex-col gap-1">
        <h2 id="sessions-networks" className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          Known networks
        </h2>
        <p className="text-xs text-muted-foreground">
          Locations are approximate and only shown to you; they never decide trust.
        </p>
        <LoadState loading={networks.value === null} failed={networks.failed} what="your networks" onRetry={() => void loadNetworks()}>
          <NetworkList networks={networks.value ?? []} onAnswer={answer} busy={busy} label="Your networks" />
        </LoadState>
      </section>
      <Suspense fallback={null}>
        <PasskeysPanel />
      </Suspense>
    </Page>
  )
}
