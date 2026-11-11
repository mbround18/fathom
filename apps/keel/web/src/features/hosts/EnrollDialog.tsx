/**
 * Enroll a host (spec 060 US1, contracts/web-ui.md "Hosts: Enroll dialog").
 *
 * Step 1: workspace (the current one by default), inventory name and profiles (chips, `base` by
 * default). Continue shows the server's preview here; Confirm asks for a passkey when it needs one.
 * Step 2: the token (masked, with Reveal), the snippet to run, the admin's instructions rendered
 * as Markdown, and how long the token stays valid. Closing step 2 asks first, because the token
 * is not shown again. A Roll from Settings → Tokens lands on step 2 too.
 */
import { FingerprintIcon, PlusIcon, WarningIcon, XIcon } from '@phosphor-icons/react'
import { Button, Input } from '@companion/ui'
import { CopyBlock, Dialog, FIELD, LABEL, Link, TermTip, useNow } from '@companion/ui/kit'
import { lazy, Suspense, useId, useState, type KeyboardEvent } from 'react'

import { RefusalText } from '@/features/teams/parts'

import { HOST_NAME, MAX_PROFILES, useEnroll, type Minted } from './enroll-store'
import { parseProfiles } from './store'
import { useWorkspaces } from './workspaces'

const Markdown = lazy(() => import('@/help/GuideMarkdown'))

function Countdown({ expiresAt }: { expiresAt: string }) {
  const now = useNow(1000)
  const end = Date.parse(expiresAt)
  const left = Math.max(0, Math.floor((end - now) / 1000))
  const text = Number.isNaN(end)
    ? expiresAt
    : left === 0
      ? 'Expired. Roll it from System Settings → Tokens for a new one.'
      : `Valid for ${String(Math.floor(left / 60))}m ${String(left % 60).padStart(2, '0')}s, once.`
  return (
    <p data-expires={expiresAt} className={`text-xs ${left === 0 ? 'text-status-err' : 'text-muted-foreground'}`}>
      <time dateTime={expiresAt}>{text}</time>
    </p>
  )
}

/** Profile chips: Enter or comma adds what is typed; each chip removes itself. */
export function ProfileChips({
  id,
  value,
  onChange,
  suggestions,
}: {
  id: string
  value: string[]
  onChange: (next: string[]) => void
  suggestions: string[]
}) {
  const [draft, setDraft] = useState('')
  const add = (text: string): void => {
    const next = [...new Set([...value, ...parseProfiles(text)])].slice(0, MAX_PROFILES)
    onChange(next)
    setDraft('')
  }
  const onKey = (e: KeyboardEvent<HTMLInputElement>): void => {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault()
      if (draft.trim()) add(draft)
    } else if (e.key === 'Backspace' && draft === '' && value.length > 0) {
      onChange(value.slice(0, -1))
    }
  }
  const offered = suggestions.filter((s) => !value.includes(s))
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <div className="flex min-w-0 flex-wrap items-center gap-1 border border-input bg-background p-1 focus-within:border-accent-neon">
        {value.map((p) => (
          <span key={p} data-profile-chip={p} className="inline-flex items-center gap-0.5 border border-border bg-muted/40 pl-1.5 text-xs">
            {p}
            <button
              type="button"
              aria-label={`Remove profile ${p}`}
              className="inline-flex size-6 items-center justify-center text-muted-foreground hover:text-foreground"
              onClick={() => {
                onChange(value.filter((x) => x !== p))
              }}
            >
              <XIcon aria-hidden className="size-3" />
            </button>
          </span>
        ))}
        <input
          id={id}
          value={draft}
          autoComplete="off"
          placeholder={value.length === 0 ? 'base' : 'Add a profile'}
          className="h-6 min-w-24 flex-1 bg-transparent px-1 text-xs outline-none"
          onChange={(e) => {
            setDraft(e.target.value)
          }}
          onKeyDown={onKey}
          onBlur={() => {
            if (draft.trim()) add(draft)
          }}
        />
      </div>
      {offered.length > 0 && (
        <div className="flex min-w-0 flex-wrap gap-1" aria-label="Suggested profiles">
          {offered.map((p) => (
            <Button
              key={p}
              type="button"
              variant="ghost"
              size="xs"
              aria-label={`Add profile ${p}`}
              onClick={() => {
                add(p)
              }}
            >
              <PlusIcon aria-hidden />
              {p}
            </Button>
          ))}
        </div>
      )}
    </div>
  )
}

function Form({ suggestions, onCancel }: { suggestions: string[]; onCancel: () => void }) {
  const initial = useEnroll((s) => s.initialWorkspace)
  const submit = useEnroll((s) => s.submit)
  const { operable, all, refusal } = useWorkspaces()
  const [workspace, setWorkspace] = useState<string | null>(initial)
  const [name, setName] = useState('')
  const [profiles, setProfiles] = useState<string[]>(['base'])
  const wsId = useId()
  const nameId = useId()
  const profilesId = useId()

  if (refusal) return <RefusalText refusal={refusal} />
  if (all === null)
    return (
      <p role="status" className="text-xs text-muted-foreground">
        Loading workspaces…
      </p>
    )
  if (operable.length === 0)
    return (
      <div data-pane-state="empty" className="flex flex-col items-start gap-2 text-xs">
        <p>There is no workspace you can enroll a host into yet.</p>
        <Link to="/teams" className="text-accent-neon underline underline-offset-2" onClick={onCancel}>
          Create a workspace in Teams
        </Link>
      </div>
    )

  const chosen = operable.find((w) => w.id === workspace) ?? operable[0]
  const trimmed = name.trim()
  const nameError = trimmed !== '' && !HOST_NAME.test(trimmed)
  const ready = chosen !== undefined && trimmed !== '' && !nameError && profiles.length > 0
  return (
    <form
      id="enroll-form"
      aria-label="Enroll host"
      className="flex min-w-0 flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        if (ready) submit(chosen.id, trimmed, profiles)
      }}
    >
      <div className="flex min-w-0 flex-col gap-1">
        <label htmlFor={wsId} className={LABEL}>
          Workspace
        </label>
        <select
          id={wsId}
          className={`${FIELD} w-full`}
          value={chosen?.id ?? ''}
          onChange={(e) => {
            setWorkspace(e.target.value)
          }}
        >
          {operable.map((w) => (
            <option key={w.id} value={w.id}>
              {w.system ? `${w.slug} (System)` : w.slug}
            </option>
          ))}
        </select>
      </div>
      <div className="flex min-w-0 flex-col gap-1">
        <label htmlFor={nameId} className={LABEL}>
          Inventory name
        </label>
        <Input
          id={nameId}
          value={name}
          autoComplete="off"
          spellCheck={false}
          placeholder="web-01"
          aria-invalid={nameError}
          aria-describedby={`${nameId}-hint`}
          className="font-mono"
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
        <p id={`${nameId}-hint`} className={`text-xs ${nameError ? 'text-status-err' : 'text-muted-foreground'}`}>
          {nameError
            ? 'Letters, digits, dot, underscore and dash only, starting with a letter or digit.'
            : 'The host’s name in the inventory, as Ansible sees it.'}
        </p>
      </div>
      <div className="flex min-w-0 flex-col gap-1">
        <div className="flex min-w-0 items-center gap-1.5">
          <label htmlFor={profilesId} className={LABEL}>
            Profiles
          </label>
          <TermTip term="profile" />
        </div>
        <ProfileChips id={profilesId} value={profiles} onChange={setProfiles} suggestions={suggestions} />
      </div>
    </form>
  )
}

function PreviewBody() {
  const preview = useEnroll((s) => s.preview)
  if (!preview) return null
  return (
    <section aria-label="Preview" className="flex min-w-0 flex-col gap-2 text-xs">
      <p data-preview-summary className="break-words">
        {preview.summary}
      </p>
      {preview.affected.length > 0 && (
        <ul data-affected className="min-w-0 border border-border">
          {preview.affected.map((a) => (
            <li key={a} className="border-t border-border px-2 py-0.5 font-mono break-all first:border-t-0">
              {a}
            </li>
          ))}
        </ul>
      )}
      {preview.step_up && (
        <p data-step-up className="flex items-center gap-1 text-status-warn">
          <FingerprintIcon aria-hidden className="size-3.5" />
          Needs your passkey.
        </p>
      )}
    </section>
  )
}

function Result({ minted }: { minted: Minted }) {
  return (
    <section aria-label="Enrolment token" className="flex min-w-0 flex-col gap-3">
      <p className="flex items-start gap-1.5 border border-status-warn/50 px-2 py-1.5 text-xs text-status-warn">
        <WarningIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
        Copy the token now. Keel will not show it again.
      </p>
      <p className="text-xs">
        <span className="font-mono">{minted.inventory_name}</span>
        <span className="text-muted-foreground"> with profiles </span>
        <span className="font-mono">{minted.profiles.join(', ')}</span>
      </p>
      <CopyBlock label="Token" tip={<TermTip term="enrolment-token" />} value={minted.token} masked data="enroll-token" />
      <CopyBlock label="Snippet" value={minted.snippet} data="enroll-snippet" />
      <Countdown expiresAt={minted.expires_at} />
      {minted.instructions_unavailable ? (
        <p data-instructions-unavailable className="text-xs text-muted-foreground">
          The instructions could not be loaded. The snippet uses the default template.
        </p>
      ) : (
        minted.instructions.trim() !== '' && (
          <section aria-label="Instructions" className="border-t border-border pt-2">
            <Suspense fallback={<p className="text-xs text-muted-foreground">Loading instructions…</p>}>
              <Markdown text={minted.instructions} />
            </Suspense>
          </section>
        )
      )}
    </section>
  )
}

export function EnrollDialog({ suggestions = [] }: { suggestions?: string[] }) {
  const phase = useEnroll((s) => s.phase)
  const mode = useEnroll((s) => s.mode)
  const refusal = useEnroll((s) => s.refusal)
  const minted = useEnroll((s) => s.minted)
  const confirm = useEnroll((s) => s.confirm)
  const back = useEnroll((s) => s.back)
  const close = useEnroll((s) => s.close)
  const [warning, setWarning] = useState(false)

  // While the passkey prompt is up, this dialog steps aside so only one dialog holds focus.
  const open = phase !== 'closed' && phase !== 'stepping'
  const tryClose = (): void => {
    if (phase === 'result' && !warning) setWarning(true)
    else {
      setWarning(false)
      close()
    }
  }

  const title = phase === 'result' ? 'Host token' : mode === 'roll' ? 'Roll enrolment token' : 'Enroll host'
  const description =
    phase === 'form'
      ? 'Keel mints a one-time token. Run the snippet on the host to enroll it.'
      : phase === 'asking'
        ? 'Asking Keel what this changes…'
        : phase === 'confirming'
          ? 'Minting the token…'
          : phase === 'refused'
            ? 'Keel refused this. Nothing was changed.'
            : undefined

  let footer
  if (warning)
    footer = (
      <>
        <Button
          variant="ghost"
          onClick={() => {
            setWarning(false)
          }}
        >
          Keep open
        </Button>
        <Button variant="destructive" onClick={tryClose}>
          Close anyway
        </Button>
      </>
    )
  else if (phase === 'form')
    footer = (
      <>
        <Button variant="ghost" onClick={tryClose}>
          Cancel
        </Button>
        <Button type="submit" form="enroll-form">
          Continue
        </Button>
      </>
    )
  else if (phase === 'preview' || phase === 'asking' || phase === 'confirming')
    footer = (
      <>
        <Button variant="ghost" onClick={mode === 'roll' ? tryClose : back}>
          {mode === 'roll' ? 'Cancel' : 'Back'}
        </Button>
        <Button disabled={phase !== 'preview'} onClick={() => void confirm()}>
          Confirm
        </Button>
      </>
    )
  else if (phase === 'refused')
    footer = (
      <>
        <Button variant="ghost" onClick={tryClose}>
          Close
        </Button>
        {mode === 'enroll' && <Button onClick={back}>Back</Button>}
      </>
    )
  else if (phase === 'result') footer = <Button onClick={tryClose}>Done</Button>

  return (
    <Dialog
      open={open}
      onClose={tryClose}
      title={title}
      {...(description === undefined ? {} : { description })}
      footer={footer}
      size={phase === 'result' ? 'lg' : 'md'}
      data-enroll-dialog={phase}
    >
      {warning ? (
        <p role="alert" data-close-warning className="flex items-start gap-1.5 text-xs text-status-warn">
          <WarningIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
          The token will not be shown again. If you have not copied it, you will need to roll a new one.
        </p>
      ) : (
        <>
          {phase === 'form' && <Form suggestions={suggestions} onCancel={close} />}
          {(phase === 'preview' || phase === 'confirming') && <PreviewBody />}
          {phase === 'refused' && refusal && <RefusalText refusal={refusal} />}
          {phase === 'result' && minted && <Result minted={minted} />}
        </>
      )}
    </Dialog>
  )
}
