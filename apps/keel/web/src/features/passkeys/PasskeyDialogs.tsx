/**
 * The enrol and step-up dialogs (spec 055 T030). Loaded only once a dialog opens; the state and
 * the ceremony live in ./store.ts.
 */
import { FingerprintIcon, KeyIcon } from '@phosphor-icons/react'
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
import { useId, useState, type SyntheticEvent } from 'react'

import { usePasskeys, type PasskeyPhase } from './store'

const BUSY: Partial<Record<PasskeyPhase, string>> = {
  server: 'Waiting for Keel…',
  authenticator: 'Follow your browser’s passkey prompt…',
}

function Status() {
  const phase = usePasskeys((s) => s.phase)
  const error = usePasskeys((s) => s.error)
  if (phase === 'refused' && error)
    return (
      <p role="alert" data-passkey-error className="border border-status-err/40 px-2 py-1 text-xs text-status-err">
        {error}
      </p>
    )
  const busy = BUSY[phase]
  if (busy)
    return (
      <p role="status" className="text-xs text-muted-foreground">
        {busy}
      </p>
    )
  return null
}

function EnrolDialog({ required }: { required: boolean }) {
  const phase = usePasskeys((s) => s.phase)
  const enrol = usePasskeys((s) => s.enrol)
  const close = usePasskeys((s) => s.close)
  const [name, setName] = useState('')
  const nameId = useId()
  const busy = phase === 'server' || phase === 'authenticator'

  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    if (name.trim() && !busy) void enrol(name)
  }

  if (phase === 'done')
    return (
      <>
        <DialogHeader>
          <DialogTitle>Passkey registered</DialogTitle>
          <DialogDescription>Keel will ask for it before risky actions.</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button onClick={close}>Done</Button>
        </DialogFooter>
      </>
    )

  return (
    <form onSubmit={submit} className="flex flex-col gap-4">
      <DialogHeader>
        <DialogTitle>Register a passkey</DialogTitle>
        <DialogDescription>
          {required
            ? 'Keel needs a passkey on your account before you can approve or run risky changes.'
            : 'A passkey proves it is you before risky actions, with your device’s lock or a security key.'}
        </DialogDescription>
      </DialogHeader>
      <div className="flex flex-col gap-1">
        <label htmlFor={nameId} className="text-xs">
          Name
        </label>
        <Input
          id={nameId}
          value={name}
          maxLength={64}
          placeholder="Work laptop"
          autoComplete="off"
          onChange={(e) => setName(e.target.value)}
        />
      </div>
      <Status />
      <DialogFooter>
        <Button type="button" variant="ghost" onClick={close}>
          {required ? 'Later' : 'Cancel'}
        </Button>
        <Button type="submit" disabled={!name.trim() || busy}>
          <KeyIcon aria-hidden />
          Register
        </Button>
      </DialogFooter>
    </form>
  )
}

function StepUpDialog({ summary }: { summary: string | null }) {
  const phase = usePasskeys((s) => s.phase)
  const verify = usePasskeys((s) => s.verify)
  const close = usePasskeys((s) => s.close)
  const busy = phase === 'server' || phase === 'authenticator'

  if (phase === 'done')
    return (
      <>
        <DialogHeader>
          <DialogTitle>Verified</DialogTitle>
          <DialogDescription>You can carry on with risky actions for a while.</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button onClick={close}>Done</Button>
        </DialogFooter>
      </>
    )

  return (
    <>
      <DialogHeader>
        <DialogTitle>Verify it’s you</DialogTitle>
        <DialogDescription>{summary ?? 'Use your passkey to confirm it is you.'}</DialogDescription>
      </DialogHeader>
      <Status />
      <DialogFooter>
        <Button type="button" variant="ghost" onClick={close}>
          Cancel
        </Button>
        <Button disabled={busy} onClick={() => void verify()}>
          <FingerprintIcon aria-hidden />
          Use passkey
        </Button>
      </DialogFooter>
    </>
  )
}

export default function PasskeyDialogs() {
  const dialog = usePasskeys((s) => s.dialog)
  const close = usePasskeys((s) => s.close)
  return (
    <Dialog
      open={dialog !== null}
      onOpenChange={(open) => {
        if (!open) close()
      }}
    >
      <DialogContent data-passkey-dialog={dialog?.kind} className="sm:max-w-md">
        {dialog?.kind === 'enrol' && <EnrolDialog required={dialog.required} />}
        {dialog?.kind === 'step_up' && <StepUpDialog summary={dialog.summary} />}
      </DialogContent>
    </Dialog>
  )
}
