/**
 * The restricted-view banner and the home/public question (spec 052, FR-013), and the admin notice
 * when client addresses are probably hidden behind an untrusted proxy (FR-021). Loaded only when
 * one of them has something to say.
 *
 * Answering never trusts the network (FR-006): "home" starts the maturity clock, "public" keeps it
 * restricted. The server records and audits the answer; this re-reads `/me` afterwards.
 */
import { WarningIcon } from '@phosphor-icons/react'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@companion/ui'
import { useState } from 'react'

import { Link } from '@/app/Link'
import { useSession } from '@/app/session-store'
import type { NetworkAnswer } from '@/protocol/generated'

import { answerNetwork } from './api'
import { untrustedText } from './model'

function Question({ network, onDone }: { network: string; onDone: () => void }) {
  const csrf = useSession((s) => s.me?.csrf_token ?? '')
  const refreshMe = useSession((s) => s.refreshMe)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const answer = async (a: NetworkAnswer): Promise<void> => {
    setBusy(true)
    setError(null)
    const r = await answerNetwork(csrf, network, a)
    setBusy(false)
    if (!r.ok) {
      setError('The answer was not saved. Try again.')
      return
    }
    await refreshMe()
    onDone()
  }

  return (
    <Dialog open onOpenChange={(o) => !o && onDone()}>
      <DialogContent data-testid="network-question">
        <DialogHeader>
          <DialogTitle className="font-display tracking-wider">Is this your home network?</DialogTitle>
          <DialogDescription>
            Depths has not seen <span className="font-mono">{network}</span> for you before. Until it is trusted you can view
            stacks, logs and stats, and start, stop or restart them; anything else waits.
          </DialogDescription>
        </DialogHeader>
        <ul className="list-disc space-y-1 pl-5 text-xs text-muted-foreground">
          <li>Home: trusted after a waiting period (a week unless the operator changed it), once you have used it on two separate days.</li>
          <li>Public (a café, a hotel, someone else&apos;s Wi-Fi): stays restricted.</li>
        </ul>
        {error && (
          <p role="alert" className="text-xs text-status-err">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button variant="ghost" size="sm" disabled={busy} onClick={onDone} data-action="answer-later">
            Not now
          </Button>
          <Button variant="outline" size="sm" disabled={busy} onClick={() => void answer('public')} data-action="answer-public">
            Public
          </Button>
          <Button size="sm" disabled={busy} onClick={() => void answer('home')} data-action="answer-home">
            Home
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

export default function NetworkGate() {
  const me = useSession((s) => s.me)
  // Asked once per network per page load; "Not now" waits for the next load.
  const [asked, setAsked] = useState<string | null>(null)
  if (!me) return null
  const d = me.network
  return (
    <>
      {me.proxy_warning && (
        <div role="note" data-testid="proxy-warning" className="flex items-center gap-2 border-b border-border px-2 py-1 text-xs text-status-warn">
          <WarningIcon aria-hidden className="size-3.5 shrink-0" />
          <span>
            Every request so far came from a private address and no trusted proxy is set, so Depths may not see real client
            addresses. Set DEPTHS_TRUSTED_PROXIES if Depths runs behind a proxy.
          </span>
        </div>
      )}
      {!d.trusted && (
        <div
          role="region"
          aria-label="Restricted view"
          data-testid="restricted-banner"
          data-reason={d.reason}
          className="flex flex-wrap items-center gap-x-2 gap-y-0.5 border-b border-border px-2 py-1 text-xs text-status-warn"
        >
          <WarningIcon aria-hidden className="size-3.5 shrink-0" />
          <span className="font-medium">Restricted view.</span>
          <span className="min-w-0">{untrustedText(d)}</span>
          <span className="flex-1" />
          {d.reason !== 'public' && d.reason !== 'maturing' && asked === d.network && (
            <Button variant="ghost" size="xs" onClick={() => setAsked(null)} data-action="answer-open">
              Home or public?
            </Button>
          )}
          <Link to="/account/sessions" className="underline underline-offset-2 hover:text-foreground">
            Your sessions
          </Link>
        </div>
      )}
      {me.needs_answer && asked !== d.network && <Question network={d.network} onDone={() => setAsked(d.network)} />}
    </>
  )
}
