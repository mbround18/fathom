/**
 * The passkey gate (spec 054 T066): the admin-actions-locked banner and the passkey dialogs.
 * Adapted from Keel's `PasskeyDialogs.tsx`. Mounted by the shell for anyone signed in, loaded lazily.
 *
 * - An admin with no passkey is asked to register one; until they do, admin actions stay locked.
 * - An admin with a passkey is asked to confirm it's them once after sign-in.
 * - Cancelling either leaves the restricted view: the banner stays, with a button to try again.
 * - The same dialog answers a step-up any action asks for (`requestStepUp`, `requestFreshCheck`),
 *   and the enrolment prompt a plain user gets when an action needs a passkey.
 */
import { FingerprintIcon, KeyIcon, LockKeyIcon } from '@phosphor-icons/react'
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

import { Link } from '@/app/Link'

import { usePasskeys, type PasskeyPhase } from './store'

const BUSY: Partial<Record<PasskeyPhase, string>> = {
  server: 'Waiting for Depths…',
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
          <DialogTitle className="font-display tracking-wider">Passkey registered</DialogTitle>
          <DialogDescription>Depths will ask for it before shells, file changes and other risky actions.</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button size="sm" onClick={close}>
            Done
          </Button>
        </DialogFooter>
      </>
    )

  return (
    <form onSubmit={submit} className="flex flex-col gap-4">
      <DialogHeader>
        <DialogTitle className="font-display tracking-wider">Register a passkey</DialogTitle>
        <DialogDescription>
          {required
            ? 'This needs a passkey on your account. Use your device’s screen lock or a security key.'
            : 'A passkey proves it’s you before risky actions, with your device’s screen lock or a security key.'}
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
        <Button type="button" variant="ghost" size="sm" onClick={close}>
          {required ? 'Not now' : 'Cancel'}
        </Button>
        <Button type="submit" size="sm" disabled={!name.trim() || busy} data-action="passkey-register">
          <KeyIcon aria-hidden />
          Register
        </Button>
      </DialogFooter>
    </form>
  )
}

function VerifyDialog({ summary }: { summary: string | null }) {
  const phase = usePasskeys((s) => s.phase)
  const verify = usePasskeys((s) => s.verify)
  const close = usePasskeys((s) => s.close)
  const busy = phase === 'server' || phase === 'authenticator'

  if (phase === 'done')
    return (
      <>
        <DialogHeader>
          <DialogTitle className="font-display tracking-wider">Confirmed</DialogTitle>
          <DialogDescription>Admin actions are unlocked for this session.</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button size="sm" onClick={close}>
            Done
          </Button>
        </DialogFooter>
      </>
    )

  return (
    <>
      <DialogHeader>
        <DialogTitle className="font-display tracking-wider">Confirm it’s you</DialogTitle>
        <DialogDescription>
          {summary === null
            ? 'Use your passkey to unlock admin actions for this session.'
            : `${summary} needs a fresh passkey check.`}
        </DialogDescription>
      </DialogHeader>
      <Status />
      <DialogFooter>
        <Button type="button" variant="ghost" size="sm" onClick={close}>
          {summary === null ? 'Not now' : 'Cancel'}
        </Button>
        <Button size="sm" disabled={busy} onClick={() => void verify()} data-action="passkey-verify">
          <FingerprintIcon aria-hidden />
          Use passkey
        </Button>
      </DialogFooter>
    </>
  )
}

function Banner() {
  const v = usePasskeys((s) => s.verification)
  const openEnrol = usePasskeys((s) => s.openEnrol)
  const openVerify = usePasskeys((s) => s.openVerify)
  if (!v?.required || v.verified) return null
  const enrol = !v.enrolled || v.recovering
  return (
    <div
      role="region"
      aria-label="Admin actions locked"
      data-testid="passkey-banner"
      data-state={enrol ? 'enrol' : 'verify'}
      className="flex flex-wrap items-center gap-x-2 gap-y-0.5 border-b border-border px-2 py-1 text-xs text-status-warn"
    >
      <LockKeyIcon aria-hidden className="size-3.5 shrink-0" />
      <span className="font-medium">Admin actions locked.</span>
      <span className="min-w-0">
        {enrol ? 'Register a passkey to unlock them.' : 'Confirm it’s you with your passkey to unlock them.'}
      </span>
      <span className="flex-1" />
      <Button
        variant="ghost"
        size="xs"
        onClick={() => (enrol ? openEnrol(true) : openVerify())}
        data-action={enrol ? 'passkey-enrol-open' : 'passkey-verify-open'}
      >
        {enrol ? 'Register a passkey' : 'Confirm it’s you'}
      </Button>
      <Link to="/account/sessions" className="underline underline-offset-2 hover:text-foreground">
        Your passkeys
      </Link>
    </div>
  )
}

export default function PasskeyGate() {
  const v = usePasskeys((s) => s.verification)
  const dialog = usePasskeys((s) => s.dialog)
  const prompted = usePasskeys((s) => s.prompted)
  const close = usePasskeys((s) => s.close)

  useEffect(() => {
    usePasskeys.getState().refreshVerification()
  }, [])

  // Ask once per session, right after sign-in; after that the banner's button asks again.
  useEffect(() => {
    if (!v?.required || v.verified || prompted || dialog !== null) return
    const s = usePasskeys.getState()
    if (v.enrolled && !v.recovering) s.openVerify()
    else {
      usePasskeys.setState({ prompted: true })
      s.openEnrol(true)
    }
  }, [v, prompted, dialog])

  return (
    <>
      <Banner />
      <Dialog
        open={dialog !== null}
        onOpenChange={(open) => {
          if (!open) close()
        }}
      >
        <DialogContent data-passkey-dialog={dialog?.kind} className="sm:max-w-md">
          {dialog?.kind === 'enrol' && <EnrolDialog required={dialog.required} />}
          {dialog?.kind === 'verify' && <VerifyDialog summary={null} />}
          {dialog?.kind === 'step_up' && <VerifyDialog summary={dialog.summary} />}
        </DialogContent>
      </Dialog>
    </>
  )
}
