/**
 * Your passkeys (spec 054 T067), on the sessions page. Adapted from Keel's `PasskeysPage.tsx`.
 *
 * - The list shows name, when it was added and when it was last used.
 * - Adding opens the shell's enrol dialog (./PasskeyGate.tsx).
 * - Removing previews first (`PreviewPasskeyRemove`), then confirms, with a passkey check when the
 *   preview asks for one. The last passkey cannot be removed: that is what recovery is for.
 * - A recovery code (from `depths passkey-recovery`) lets someone who lost every passkey register
 *   a new one.
 */
import { FingerprintIcon, KeyIcon, PlusIcon, TrashIcon } from '@phosphor-icons/react'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
} from '@companion/ui'
import { useEffect, useId, useState, type SyntheticEvent } from 'react'

import type { PasskeyInfo, PasskeyRequest, Preview } from '@/protocol/generated'

import { stepUpAssertion } from './api'
import { stepUpText } from './messages'
import { usePasskeys } from './store'

const DATE = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })

function When({ iso }: { iso: string }) {
  const d = new Date(iso)
  return <time dateTime={iso}>{Number.isNaN(d.getTime()) ? iso : DATE.format(d)}</time>
}

type Removal =
  | { passkey: PasskeyInfo; phase: 'previewing' }
  | { passkey: PasskeyInfo; phase: 'ready' | 'removing'; preview: Preview; stepUp: PasskeyRequest | null; message: string | null }
  | { passkey: PasskeyInfo; phase: 'refused'; message: string }

function RemoveDialog({ removal, onClose, onConfirm }: { removal: Removal | null; onClose: () => void; onConfirm: () => void }) {
  const removing = removal?.phase === 'removing'
  return (
    <Dialog
      open={removal !== null}
      onOpenChange={(o) => {
        if (!o && !removing) onClose()
      }}
    >
      <DialogContent data-remove-dialog={removal?.phase} showCloseButton={!removing} className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="font-display tracking-wider">Remove passkey</DialogTitle>
          <DialogDescription>
            {removal?.phase === 'refused' ? `“${removal.passkey.name}” was not removed.` : `“${removal?.passkey.name ?? ''}” will stop working with Depths.`}
          </DialogDescription>
        </DialogHeader>
        {removal?.phase === 'previewing' && (
          <p role="status" className="text-xs text-muted-foreground">
            Checking with Depths…
          </p>
        )}
        {(removal?.phase === 'ready' || removal?.phase === 'removing') && removal.stepUp && (
          <p className="flex items-start gap-1 text-xs text-muted-foreground">
            <FingerprintIcon aria-hidden className="mt-0.5 size-3 shrink-0" />
            <span>Depths will ask for one of your passkeys to confirm.</span>
          </p>
        )}
        {removal?.phase === 'refused' && (
          <p role="alert" data-refusal className="border border-status-err/40 px-2 py-1 text-xs text-status-err">
            {removal.message}
          </p>
        )}
        {removal?.phase === 'ready' && removal.message && (
          <p role="alert" data-refusal className="border border-status-warn/60 px-2 py-1 text-xs text-status-warn">
            {removal.message}
          </p>
        )}
        {removing && (
          <p role="status" className="text-xs text-muted-foreground">
            Removing…
          </p>
        )}
        <DialogFooter>
          {removal?.phase === 'refused' ? (
            <Button size="sm" onClick={onClose}>
              OK
            </Button>
          ) : (
            <>
              <Button variant="ghost" size="sm" disabled={removing} onClick={onClose}>
                Cancel
              </Button>
              <Button
                variant="destructive"
                size="sm"
                disabled={removal?.phase !== 'ready'}
                onClick={onConfirm}
                data-confirm
              >
                <TrashIcon aria-hidden />
                Remove
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function Recovery() {
  const recover = usePasskeys((s) => s.recover)
  const openEnrol = usePasskeys((s) => s.openEnrol)
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const id = useId()
  const valid = /^[A-Za-z0-9]{10,32}$/.test(code.trim())

  const submit = async (e: SyntheticEvent): Promise<void> => {
    e.preventDefault()
    if (!valid || busy) return
    setBusy(true)
    setError(null)
    const message = await recover(code)
    setBusy(false)
    if (message !== null) {
      setError(message)
      return
    }
    setCode('')
    openEnrol(true)
  }

  return (
    <form onSubmit={(e) => void submit(e)} data-testid="passkey-recovery" className="flex min-w-0 flex-col gap-1">
      <label htmlFor={id} className="text-xs text-muted-foreground">
        Lost every passkey? Enter the recovery code your operator gave you, then register a new one.
      </label>
      <div className="flex min-w-0 gap-2">
        <Input
          id={id}
          value={code}
          maxLength={32}
          autoComplete="one-time-code"
          spellCheck={false}
          placeholder="Recovery code"
          className="min-w-0 flex-1 font-mono"
          onChange={(e) => setCode(e.target.value)}
        />
        <Button type="submit" variant="outline" size="sm" disabled={!valid || busy} data-action="passkey-recover">
          Use code
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-xs text-status-err">
          {error}
        </p>
      )}
    </form>
  )
}

export default function PasskeysPanel() {
  const list = usePasskeys((s) => s.list)
  const loadList = usePasskeys((s) => s.loadList)
  const openEnrol = usePasskeys((s) => s.openEnrol)
  const previewRemove = usePasskeys((s) => s.previewRemove)
  const confirmRemove = usePasskeys((s) => s.confirmRemove)
  const [removal, setRemoval] = useState<Removal | null>(null)

  useEffect(() => {
    loadList()
  }, [loadList])

  const startRemove = async (passkey: PasskeyInfo): Promise<void> => {
    setRemoval({ passkey, phase: 'previewing' })
    const r = await previewRemove(passkey.id)
    setRemoval(
      r.ok
        ? { passkey, phase: 'ready', preview: r.preview, stepUp: r.preview.step_up ?? null, message: null }
        : { passkey, phase: 'refused', message: r.message },
    )
  }

  const confirm = async (): Promise<void> => {
    if (removal?.phase !== 'ready') return
    const { passkey, preview, stepUp } = removal
    setRemoval({ ...removal, phase: 'removing', message: null })
    let assertion
    if (stepUp) {
      const signed = await stepUpAssertion(stepUp)
      if (!signed.ok) {
        setRemoval({ passkey, phase: 'ready', preview, stepUp, message: stepUpText(signed.refusal) })
        return
      }
      assertion = signed.assertion
    }
    const r = await confirmRemove(preview, assertion)
    if (r === null) setRemoval(null)
    else if (r.stepUp) setRemoval({ passkey, phase: 'ready', preview, stepUp: r.stepUp, message: r.message })
    else setRemoval({ passkey, phase: 'refused', message: r.message })
  }

  const last = list?.length === 1

  return (
    <section aria-labelledby="sessions-passkeys" data-testid="passkeys-panel" className="flex min-w-0 flex-col gap-1">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <h2 id="sessions-passkeys" className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          Passkeys
        </h2>
        <span className="flex-1" />
        <Button variant="outline" size="sm" onClick={() => openEnrol(false)} data-action="passkey-add">
          <PlusIcon aria-hidden />
          Add passkey
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Depths asks for one of these before shells, file changes and other risky actions. Keep more than one, in case
        you lose a device.
      </p>
      {list === null ? (
        <p className="text-xs text-muted-foreground">Loading…</p>
      ) : list.length === 0 ? (
        <p data-testid="passkeys-empty" className="text-xs text-muted-foreground">
          No passkeys yet.
        </p>
      ) : (
        <ul aria-label="Your passkeys" className="min-w-0 border border-border">
          {list.map((p) => (
            <li key={p.id} data-passkey={p.id} className="flex min-w-0 items-center gap-2 border-t border-border px-2 py-1.5 first:border-t-0">
              <KeyIcon aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
              <div className="flex min-w-0 flex-1 flex-col">
                <span data-field="name" className="truncate text-sm">
                  {p.name}
                </span>
                <span className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
                  <span data-field="created">
                    Added <When iso={p.created} />
                  </span>
                  <span data-field="last-used">
                    {p.last_used === null ? (
                      'Never used'
                    ) : (
                      <>
                        Last used <When iso={p.last_used} />
                      </>
                    )}
                  </span>
                </span>
              </div>
              <Button
                variant="ghost"
                size="sm"
                disabled={last || removal !== null}
                title={last ? 'Your last passkey cannot be removed. Add another first.' : undefined}
                aria-label={`Remove ${p.name}`}
                onClick={() => void startRemove(p)}
              >
                <TrashIcon aria-hidden />
                <span className="hidden sm:inline">Remove</span>
              </Button>
            </li>
          ))}
        </ul>
      )}
      {last && <p className="text-xs text-muted-foreground">Your only passkey cannot be removed. Add another first.</p>}
      <Recovery />
      <RemoveDialog removal={removal} onClose={() => setRemoval(null)} onConfirm={() => void confirm()} />
    </section>
  )
}
