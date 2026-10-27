/**
 * Account → Passkeys (spec 055): the person's own passkeys, with add and remove. The state and
 * the protocol live in ./list-store.ts; adding reuses the enrol dialog (./PasskeyDialogs.tsx,
 * mounted by the shell), and removal's step-up reuses the step-up dialog.
 */
import { FingerprintIcon, KeyIcon, PlusIcon, TrashIcon, WarningIcon } from '@phosphor-icons/react'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@companion/ui'
import { useEffect } from 'react'

import type { PasskeyInfo } from '@/protocol/wire'

import { usePasskeyList, type Removal } from './list-store'

const DATE = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })

function When({ iso }: { iso: string }) {
  const d = new Date(iso)
  return <time dateTime={iso}>{Number.isNaN(d.getTime()) ? iso : DATE.format(d)}</time>
}

function Row({ passkey }: { passkey: PasskeyInfo }) {
  const startRemove = usePasskeyList((s) => s.startRemove)
  const busy = usePasskeyList((s) => s.removal !== null)
  return (
    <li
      data-passkey={passkey.id}
      className="flex min-w-0 items-center gap-2 border-t border-border px-2 py-2 first:border-t-0"
    >
      <KeyIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span data-field="name" className="truncate text-sm">
          {passkey.name}
        </span>
        <span className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
          <span data-field="created">
            Added <When iso={passkey.created} />
          </span>
          <span data-field="last-used">
            {passkey.last_used === null ? (
              'Never used'
            ) : (
              <>
                Last used <When iso={passkey.last_used} />
              </>
            )}
          </span>
        </span>
      </div>
      <Button
        variant="ghost"
        size="sm"
        disabled={busy}
        aria-label={`Remove ${passkey.name}`}
        onClick={() => void startRemove(passkey)}
      >
        <TrashIcon aria-hidden />
        <span className="hidden sm:inline">Remove</span>
      </Button>
    </li>
  )
}

function List() {
  const status = usePasskeyList((s) => s.status)
  const items = usePasskeyList((s) => s.items)
  const error = usePasskeyList((s) => s.error)
  const load = usePasskeyList((s) => s.load)

  if (status === 'idle' || status === 'loading')
    return (
      <p role="status" data-pane-state="loading" className="border border-border p-2 text-xs text-muted-foreground">
        Loading passkeys…
      </p>
    )
  if (status === 'error')
    return (
      <div
        role="alert"
        data-pane-state="error"
        className="flex flex-wrap items-center gap-2 border border-status-err/40 p-2 text-xs text-status-err"
      >
        <span className="min-w-0 flex-1 break-words">{error ?? 'The passkeys could not be loaded.'}</span>
        <Button size="sm" variant="outline" onClick={() => void load()}>
          Retry
        </Button>
      </div>
    )
  if (items.length === 0)
    return (
      <p data-pane-state="empty" className="border border-border p-2 text-xs text-muted-foreground">
        You have no passkeys yet. Add one to approve and run risky changes.
      </p>
    )
  return (
    <ul aria-label="Your passkeys" className="min-w-0 border border-border">
      {items.map((p) => (
        <Row key={p.id} passkey={p} />
      ))}
    </ul>
  )
}

function RemoveBody({ removal }: { removal: Removal }) {
  const confirmRemove = usePasskeyList((s) => s.confirmRemove)
  const cancelRemove = usePasskeyList((s) => s.cancelRemove)
  const { passkey, phase, preview, error } = removal

  if (phase === 'refused')
    return (
      <>
        <DialogDescription>“{passkey.name}” was not removed.</DialogDescription>
        <p
          role="alert"
          data-refusal
          className="border border-status-warn/60 bg-status-warn/10 p-1.5 text-xs break-words text-status-warn"
        >
          {error}
        </p>
        <DialogFooter>
          <Button onClick={cancelRemove}>OK</Button>
        </DialogFooter>
      </>
    )

  if (phase === 'previewing' || !preview)
    return (
      <>
        <DialogDescription>“{passkey.name}”</DialogDescription>
        <p role="status" data-pane-state="loading" className="text-xs text-muted-foreground">
          Checking with Keel…
        </p>
        <DialogFooter>
          <Button variant="ghost" onClick={cancelRemove}>
            Cancel
          </Button>
        </DialogFooter>
      </>
    )

  const removing = phase === 'removing'
  return (
    <>
      <DialogDescription>{preview.summary}</DialogDescription>
      {preview.affected.length > 0 && (
        <ul aria-label="Affects" className="border border-border bg-background/40 px-2 text-xs">
          {preview.affected.map((a) => (
            <li key={a} className="border-t border-border py-1 break-words first:border-t-0">
              {a}
            </li>
          ))}
        </ul>
      )}
      <p className="flex items-start gap-1 text-xs text-muted-foreground">
        <WarningIcon aria-hidden className="mt-0.5 size-3 shrink-0" />
        <span>You will no longer be able to use it with Keel. Your other sessions may be signed out.</span>
      </p>
      {preview.step_up && (
        <p className="flex items-start gap-1 text-xs text-muted-foreground">
          <FingerprintIcon aria-hidden className="mt-0.5 size-3 shrink-0" />
          <span>Keel will ask for one of your passkeys to confirm.</span>
        </p>
      )}
      {removing && (
        <p role="status" className="text-xs text-muted-foreground">
          Removing…
        </p>
      )}
      <DialogFooter>
        <Button variant="ghost" disabled={removing} onClick={cancelRemove}>
          Cancel
        </Button>
        <Button variant="destructive" disabled={removing} onClick={() => void confirmRemove()}>
          <TrashIcon aria-hidden />
          Remove
        </Button>
      </DialogFooter>
    </>
  )
}

/** The removal's dialog. Hidden while the step-up dialog asks for the passkey. */
function RemoveDialog() {
  const removal = usePasskeyList((s) => s.removal)
  const cancelRemove = usePasskeyList((s) => s.cancelRemove)
  const open = removal !== null && removal.phase !== 'verifying'
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        // Closing mid-Confirm would hide the answer; the buttons are disabled then too.
        if (!o && removal?.phase !== 'removing') cancelRemove()
      }}
    >
      <DialogContent
        data-remove-dialog={removal?.phase}
        showCloseButton={removal?.phase !== 'removing'}
        className="sm:max-w-md"
      >
        <DialogHeader>
          <DialogTitle>Remove passkey</DialogTitle>
        </DialogHeader>
        {removal && <RemoveBody removal={removal} />}
      </DialogContent>
    </Dialog>
  )
}

export default function PasskeysPage() {
  const load = usePasskeyList((s) => s.load)
  const reset = usePasskeyList((s) => s.reset)
  const add = usePasskeyList((s) => s.add)
  const addError = usePasskeyList((s) => s.addError)

  useEffect(() => {
    void load()
    return reset
  }, [load, reset])

  return (
    <section
      data-page="passkeys"
      aria-labelledby="passkeys-title"
      className="flex max-w-2xl min-w-0 flex-col gap-3 p-3 sm:p-4"
    >
      <header className="flex flex-wrap items-center gap-2">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <h1 id="passkeys-title" className="font-display text-sm font-semibold tracking-wider uppercase">
            Passkeys
          </h1>
          <p className="text-xs text-muted-foreground">
            Keel asks for one of these before risky actions. Keep more than one, in case you lose a device.
          </p>
        </div>
        <Button size="sm" onClick={() => void add()}>
          <PlusIcon aria-hidden />
          Add passkey
        </Button>
      </header>
      {addError && (
        <p role="alert" data-add-error className="border border-status-err/40 px-2 py-1 text-xs text-status-err">
          {addError}
        </p>
      )}
      <List />
      <RemoveDialog />
    </section>
  )
}
