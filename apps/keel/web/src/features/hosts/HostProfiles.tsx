/**
 * A host's profiles (spec 068 US3, contracts/ui.md "Host page", T040).
 *
 * - The Profiles card in the host panel: the assigned profiles as chips, the host's own ceiling
 *   under them, and a warning icon with text on any chip outside it (never colour alone).
 * - Edit profiles (workspace admin): the chips editor, then a preview of what is added and
 *   removed, the certificate principals after, the converge steps after and any profile the
 *   runner will refuse. Confirm asks for a passkey.
 * - The inventory's suggestions ("+ k3s-agent from k3s_agents") with Accept, which opens the same
 *   preview with the suggestion applied, and Dismiss.
 * - On the Hosts list, a dot after the Profiles cell when a host has open suggestions.
 */
import { FingerprintIcon, MinusIcon, PencilSimpleIcon, PlusIcon, WarningIcon } from '@phosphor-icons/react'
import { Button, Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@companion/ui'
import { ChipList, Dialog, H3, LABEL, LoadState, RefusalText, TermTip } from '@companion/ui/kit'
import { useEffect, useId, useMemo } from 'react'

import { useSession } from '@/app/session-store'
import { useConverge } from '@/features/converge/store'
import { profileOptions } from '@/features/repository/catalog'
import { useCatalog } from '@/features/repository/store'
import type { HostInfo, ProfileSuggestion } from '@/protocol/wire'

import { ProfileChips } from './EnrollDialog'
import { ceilingText, changeOf, outsideCeiling, stepLine, suggestionCount, suggestionReason } from './profiles'
import { suggestionKey, useProfiles } from './profiles-store'
import { useHosts } from './store'
import { canAdminister } from './workspaces'

const CHIP = 'inline-flex max-w-full items-center gap-1 border px-1.5 py-0.5 font-mono text-xs break-all'

/** One assigned profile; outside the ceiling it says so in words, beside the icon. */
function Chip({ profile, outside }: { profile: string; outside: boolean }) {
  return (
    <li
      data-profile={profile}
      data-outside-ceiling={outside}
      className={`${CHIP} ${outside ? 'border-status-warn/60 text-status-warn' : 'border-border bg-muted/40'}`}
    >
      {outside && <WarningIcon aria-hidden className="size-3.5 shrink-0" />}
      {profile}
      {outside && <span className="font-sans">outside ceiling</span>}
    </li>
  )
}

function SuggestionRow({ host, s, admin }: { host: HostInfo; s: ProfileSuggestion; admin: boolean }) {
  const accept = useProfiles((st) => st.accept)
  const dismiss = useProfiles((st) => st.dismiss)
  const busy = useProfiles((st) => st.dismissing === suggestionKey(s))
  const add = s.action === 'add'
  const Sign = add ? PlusIcon : MinusIcon
  return (
    <li
      data-suggestion={s.profile}
      data-suggestion-action={s.action}
      className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 border-t border-border px-2 py-1.5 first:border-t-0"
    >
      <span className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-1.5">
        <span className="inline-flex items-center gap-1 font-mono">
          <Sign aria-hidden className="size-3 shrink-0 self-center" />
          <span className="sr-only">{add ? 'Add' : 'Remove'}</span>
          {s.profile}
        </span>
        <span className="text-muted-foreground">{suggestionReason(s)}</span>
      </span>
      {admin && (
        <span className="flex shrink-0 gap-1">
          <Button
            size="xs"
            variant="outline"
            data-suggestion-accept
            aria-label={`Accept: ${add ? 'add' : 'remove'} ${s.profile}`}
            onClick={() => {
              accept(host, s)
            }}
          >
            Accept
          </Button>
          <Button
            size="xs"
            variant="ghost"
            data-suggestion-dismiss
            disabled={busy}
            aria-label={`Dismiss ${s.profile}`}
            onClick={() => {
              dismiss(s)
            }}
          >
            Dismiss
          </Button>
        </span>
      )}
    </li>
  )
}

function Suggestions({ host, admin }: { host: HostInfo; admin: boolean }) {
  const items = useProfiles((s) => s.suggestions[host.id])
  const failed = useProfiles((s) => s.suggestionsFailed[host.id])
  if (failed) return <RefusalText refusal={failed} lead="Suggestions could not be read." />
  if (!items || items.length === 0) return null
  return (
    <section aria-label="Inventory suggestions" data-suggestions={items.length} className="flex min-w-0 flex-col gap-1">
      <h4 className={LABEL}>Suggested by inventory groups</h4>
      <ul className="flex min-w-0 flex-col border border-border">
        {items.map((s) => (
          <SuggestionRow key={suggestionKey(s)} host={host} s={s} admin={admin} />
        ))}
      </ul>
    </section>
  )
}

/** The Profiles card in the host panel. */
export function ProfilesCard({ host }: { host: HostInfo }) {
  const me = useSession((s) => s.me)
  const edit = useProfiles((s) => s.edit)
  const load = useProfiles((s) => s.loadSuggestions)
  const suggestionsOff = useProfiles((s) => s.disabled)
  const convergeOff = useConverge((s) => s.disabled)
  const off = suggestionsOff || convergeOff
  const admin = canAdminister(me, host.workspace) && host.state !== 'revoked'
  const outside = new Set(outsideCeiling(host))

  // The list's count is the live one (a sync re-reads the list, not the open host). Read the
  // suggestions only when there are some: on open, and whenever the count moves.
  const count = useHosts((s) => s.hosts.find((h) => h.id === host.id)?.suggestions) ?? host.suggestions
  useEffect(() => {
    if (count > 0) load(host)
    // eslint-disable-next-line react-hooks/exhaustive-deps -- by host and its count, not identity
  }, [host.id, count, load])

  return (
    <section aria-label="Profiles" data-profiles-card className="flex min-w-0 flex-col gap-1.5">
      <div className="flex min-h-7 flex-wrap items-center gap-x-2 gap-y-1">
        <h3 className={`${LABEL} inline-flex items-center gap-1`}>
          Profiles
          <TermTip term="profile" />
        </h3>
        {admin && !off && (
          <Button
            size="xs"
            variant="outline"
            className="ml-auto"
            data-edit-profiles
            onClick={() => {
              edit(host)
            }}
          >
            <PencilSimpleIcon aria-hidden />
            Edit
          </Button>
        )}
      </div>
      {host.profiles.length === 0 ? (
        <p className="text-muted-foreground">No profiles assigned. Keel runs nothing here.</p>
      ) : (
        <ul aria-label="Assigned profiles" className="flex min-w-0 flex-wrap gap-1">
          {host.profiles.map((p) => (
            <Chip key={p} profile={p} outside={outside.has(p)} />
          ))}
        </ul>
      )}
      <p data-ceiling className="text-muted-foreground">
        {ceilingText(host.allowed_profiles)}
      </p>
      {outside.size > 0 && host.allowed_profiles.length > 0 && (
        <p data-outside-note className="flex items-start gap-1.5 text-status-warn">
          <WarningIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
          <span>
            Its runner refuses jobs for {[...outside].join(', ')}. Add {outside.size === 1 ? 'it' : 'them'} to{' '}
            <code className="font-mono">keel_runner_allowed_profiles</code> on the host, or remove{' '}
            {outside.size === 1 ? 'it' : 'them'} here.
          </span>
        </p>
      )}
      {!off && <Suggestions host={host} admin={admin} />}
    </section>
  )
}

function Diff({ add, remove }: { add: readonly string[]; remove: readonly string[] }) {
  if (add.length === 0 && remove.length === 0) return <p className="text-muted-foreground">No change yet.</p>
  return (
    <ul aria-label="Changes" className="flex min-w-0 flex-wrap gap-1">
      {add.map((p) => (
        <li key={`+${p}`} data-change="add" className={`${CHIP} border-status-ok/50`}>
          <PlusIcon aria-hidden className="size-3 shrink-0" />
          <span className="sr-only">Add</span>
          {p}
        </li>
      ))}
      {remove.map((p) => (
        <li key={`-${p}`} data-change="remove" className={`${CHIP} border-status-err/50 line-through decoration-status-err`}>
          <MinusIcon aria-hidden className="size-3 shrink-0" />
          <span className="sr-only">Remove</span>
          {p}
        </li>
      ))}
    </ul>
  )
}

function Editor({ host }: { host: HostInfo }) {
  const draft = useProfiles((s) => s.draft)
  const setDraft = useProfiles((s) => s.setDraft)
  const review = useProfiles((s) => s.review)
  const jobs = useCatalog((s) => s.jobs)
  const hosts = useHosts((s) => s.hosts)
  const id = useId()
  const offered = useMemo(
    () => [...new Set([...profileOptions(jobs ?? [], hosts.flatMap((h) => h.profiles)), ...host.allowed_profiles])].sort(),
    [jobs, hosts, host.allowed_profiles],
  )
  const { add, remove } = changeOf(host.profiles, draft)
  const outside = add.filter((p) => !host.allowed_profiles.includes(p))
  return (
    <form
      id="profiles-form"
      aria-label={`Profiles of ${host.inventory_name}`}
      className="flex min-w-0 flex-col gap-3 text-xs"
      onSubmit={(e) => {
        e.preventDefault()
        if (add.length + remove.length > 0) review()
      }}
    >
      <div className="flex min-w-0 flex-col gap-1">
        <label htmlFor={id} className={LABEL}>
          Profiles
        </label>
        <ProfileChips id={id} value={draft} onChange={setDraft} suggestions={offered} />
        <p className="text-muted-foreground">{ceilingText(host.allowed_profiles)}</p>
      </div>
      <div className="flex min-w-0 flex-col gap-1">
        <h3 className={LABEL}>Change</h3>
        <Diff add={add} remove={remove} />
        {outside.length > 0 && (
          <p data-outside-warning className="flex items-start gap-1.5 text-status-warn">
            <WarningIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
            Outside the host&apos;s ceiling: {outside.join(', ')}. Its runner will refuse their jobs.
          </p>
        )}
      </div>
    </form>
  )
}

function PreviewBody() {
  const preview = useProfiles((s) => s.preview)
  const pending = useProfiles((s) => s.pending)
  if (!preview) return <LoadState loading label="Asking Keel what this changes…" />
  return (
    <section aria-label="Preview" className="flex min-w-0 flex-col gap-3 text-xs">
      <div className="flex min-w-0 flex-col gap-1">
        <h3 className={H3}>Change</h3>
        <Diff add={preview.change.add} remove={preview.change.remove} />
      </div>
      {preview.outside_ceiling.length > 0 && (
        <p
          data-preview-outside
          className="flex items-start gap-1.5 border border-status-warn/50 px-2 py-1.5 text-status-warn"
        >
          <WarningIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
          <span>
            Outside the host&apos;s ceiling, so its runner will refuse their jobs:{' '}
            <span className="font-mono">{preview.outside_ceiling.join(', ')}</span>. They are not in its certificate.
          </span>
        </p>
      )}
      <div className="flex min-w-0 flex-col gap-1">
        <h3 className={H3}>Certificate principals after</h3>
        <ChipList label="Principals after" items={preview.principals_after} max={8} />
      </div>
      <div className="flex min-w-0 flex-col gap-1">
        <h3 className={H3}>Converge after</h3>
        {preview.plan_after.length === 0 ? (
          <p data-plan-after="empty" className="text-muted-foreground">
            No converge step: no repository plans one for these profiles.
          </p>
        ) : (
          <ol aria-label="Steps after" data-plan-after={preview.plan_after.length} className="list-decimal pl-5">
            {preview.plan_after.map((s) => (
              <li key={s.position} data-step={s.job} className="font-mono break-all">
                {stepLine(s)}
              </li>
            ))}
          </ol>
        )}
        <p className="text-muted-foreground">Nothing runs now. Converge the host afterwards to apply it.</p>
      </div>
      {pending?.step_up && (
        <p data-step-up className="flex items-center gap-1 text-status-warn">
          <FingerprintIcon aria-hidden className="size-3.5" />
          Needs your passkey.
        </p>
      )}
    </section>
  )
}

/** The profile editor and its preview, mounted once by the Hosts page. */
export function ProfilesDialog() {
  const host = useProfiles((s) => s.host)
  const phase = useProfiles((s) => s.phase)
  const draft = useProfiles((s) => s.draft)
  const refusal = useProfiles((s) => s.refusal)
  const confirm = useProfiles((s) => s.confirm)
  const back = useProfiles((s) => s.back)
  const close = useProfiles((s) => s.close)
  // While the passkey prompt is up, this dialog steps aside so only one dialog holds focus.
  const open = host !== null && phase !== 'closed' && phase !== 'stepping'
  const changed = host !== null && (() => {
    const c = changeOf(host.profiles, draft)
    return c.add.length + c.remove.length > 0
  })()

  // Each phase's buttons are keyed apart: Back must not become Preview's submit button in the
  // re-render its own click causes, or the click submits the editor again.
  let footer
  if (phase === 'edit')
    footer = (
      <>
        <Button key="cancel" variant="ghost" onClick={close}>
          Cancel
        </Button>
        <Button key="preview" type="submit" form="profiles-form" disabled={!changed} data-profiles-preview>
          Preview
        </Button>
      </>
    )
  else if (phase === 'refused')
    footer = (
      <>
        <Button key="close" variant="ghost" onClick={close}>
          Close
        </Button>
        <Button key="refused-back" type="button" onClick={back}>
          Back
        </Button>
      </>
    )
  else
    footer = (
      <>
        <Button key="back" variant="ghost" onClick={back}>
          Back
        </Button>
        <Button key="confirm" data-profiles-confirm disabled={phase !== 'preview'} onClick={() => void confirm()}>
          Set profiles
        </Button>
      </>
    )

  return (
    <Dialog
      open={open}
      onClose={close}
      title={host ? `Profiles of ${host.inventory_name}` : 'Profiles'}
      description={
        phase === 'edit'
          ? 'Keel re-issues the host’s certificate with the new profiles. Nothing runs until it is converged.'
          : phase === 'refused'
            ? 'Keel refused this. Nothing was changed.'
            : phase === 'confirming'
              ? 'Setting the profiles…'
              : undefined
      }
      footer={footer}
      size="md"
      data-profiles-dialog={phase}
    >
      {host && phase === 'edit' && <Editor host={host} />}
      {(phase === 'asking' || phase === 'preview' || phase === 'confirming') && <PreviewBody />}
      {phase === 'refused' && refusal && <RefusalText refusal={refusal} />}
    </Dialog>
  )
}

/** The Hosts list's dot: the host has open suggestions. Opens the host, where they are listed. */
export function SuggestionDot({ host }: { host: HostInfo }) {
  const show = useHosts((s) => s.show)
  if (host.suggestions === 0) return null
  const text = suggestionCount(host.suggestions)
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            data-suggestion-dot={host.suggestions}
            aria-label={`${host.inventory_name}: ${text}`}
            className="inline-flex size-6 shrink-0 items-center justify-center focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none"
            onClick={() => {
              show(host.id)
            }}
          >
            <span aria-hidden className="size-2 rounded-full bg-accent-neon" />
          </button>
        </TooltipTrigger>
        <TooltipContent>{text}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}
