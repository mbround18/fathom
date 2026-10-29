/**
 * `/admin/updates` (spec 062): every service's image update state, and what to do about it.
 *
 * - Check now, the time zone and the next check in the header (US1).
 * - While checks are off: what checking does, and a "Turn checks on" that goes through a preview.
 * - The Watchtower banner and the settings sheet (US3-US5, `./Schedule`).
 * - Filters by state and stack, then one virtualised scroller (the audit log's pattern), each
 *   service a grid row on wide screens and a card on narrow ones. Stacks with a newer image can be
 *   selected and updated together (US2, `./BatchDialogs`); a held image can be applied anyway and
 *   a rejected one offered again.
 * - Stack policies, recent runs and the registries Depths has talked to.
 *
 * Every change is a preview first, then a passkey on Confirm (054).
 */
import { ArrowClockwiseIcon, MagnifyingGlassIcon } from '@phosphor-icons/react'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@companion/ui'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useEffect, useMemo, useRef, useState } from 'react'

import { Link } from '@/app/Link'
import { useUntrusted } from '@/features/network/model'
import { formatTime } from '@/features/stacks/model'
import type { ServiceUpdate, UpdateChange, UpdatesView, UpdateSettings } from '@/protocol/generated'

import { BatchDialog, RunDialog, StepUpNote } from './BatchDialogs'

import {
  authText,
  availableText,
  countStates,
  exclusionText,
  filterServices,
  INTERVALS,
  intervalText,
  isSelf,
  isUtcZone,
  reasonOf,
  runningText,
  SELF_UPDATE_STEPS,
  STATE_LABEL,
  STATE_ORDER,
  stacksOf,
  untilText,
  updatableStacks,
  withChecks,
  type StateFilter,
} from './model'
import { Policies, RecentRuns, SettingsSheet, WatchtowerBanner, ZoneNote } from './Schedule'
import { LabelList, PolicyLine } from './ServicePolicy'
import { useUpdates, type CheckStatus } from './store'
import { UpdateStateChip } from './UpdateStateChip'

const ESTIMATED_ROW_PX = 56
const OVERSCAN = 8
const FIELD = 'h-7 min-w-0 border border-input bg-background px-1.5 text-xs outline-none focus:border-ring'
const ROW =
  'grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-2 gap-y-0.5 px-2 py-1.5 text-xs md:grid-cols-[minmax(0,12rem)_9rem_minmax(0,1fr)_minmax(0,1fr)_8rem] md:gap-y-0'
const LABEL = 'text-muted-foreground md:hidden'

export default function UpdatesPage() {
  const view = useUpdates((s) => s.view)
  const loaded = useUpdates((s) => s.loaded)
  const denied = useUpdates((s) => s.denied)
  const load = useUpdates((s) => s.load)

  useEffect(() => {
    load()
  }, [load])

  return (
    <section data-page="updates" aria-labelledby="updates-title" className="flex h-full min-w-0 flex-col">
      <Header view={view} />
      {denied !== null ? (
        <p role="status" className="p-3 text-xs text-muted-foreground">
          {denied === 'feature_disabled' ? 'Image updates are turned off on this server.' : "You can't see updates."}
        </p>
      ) : !loaded || view === null ? (
        <p role="status" data-pane-state="loading" className="p-3 text-xs text-muted-foreground">
          Loading updates…
        </p>
      ) : (
        <Body view={view} />
      )}
      <ChangeDialog />
      <BatchDialog />
      <RunDialog />
    </section>
  )
}

function Header({ view }: { view: UpdatesView | null }) {
  const check = useUpdates((s) => s.check)
  const checkNow = useUpdates((s) => s.checkNow)
  const blocked = useUntrusted()
  const on = view?.settings.checks_enabled === true
  return (
    <div className="flex min-w-0 shrink-0 flex-col gap-1 border-b border-border px-2 py-1.5">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <h1 id="updates-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Updates
        </h1>
        {view !== null && (
          <span data-testid="updates-schedule" className="min-w-0 text-xs text-muted-foreground">
            {on
              ? `Checks ${intervalText(view.settings.interval_secs)}${view.next_check ? `, next ${untilText(view.next_check)}` : ''}`
              : 'Checks are off'}
            <span className="hidden sm:inline"> · {view.zone}</span>
            {view.next_window && (
              <span data-field="next-window" className="hidden sm:inline">
                {' '}
                · Auto window {untilText(view.next_window)}
              </span>
            )}
          </span>
        )}
        <span className="flex-1" />
        {on && blocked === null && (
          <Button
            variant="outline"
            size="sm"
            data-action="check-now"
            disabled={check.kind === 'sending'}
            onClick={checkNow}
          >
            <ArrowClockwiseIcon aria-hidden />
            Check now
          </Button>
        )}
      </div>
      <CheckLine check={check} />
    </div>
  )
}

function CheckLine({ check }: { check: CheckStatus }) {
  switch (check.kind) {
    case 'idle':
    case 'sending':
      return null
    case 'started':
      return (
        <p role="status" data-check="started" className="text-xs text-muted-foreground">
          Checking every image now. States change as each registry answers.
        </p>
      )
    case 'busy':
      return (
        <p role="status" data-check="busy" className="text-xs text-status-warn">
          A check is already running. Try again in {String(Math.max(1, Math.ceil(check.retryAfterSecs / 60)))} min.
        </p>
      )
    case 'refused':
      return (
        <p role="alert" data-check="refused" className="text-xs text-status-err">
          {check.reason}
        </p>
      )
  }
}

function Body({ view }: { view: UpdatesView }) {
  if (view.available.kind === 'unreadable')
    return (
      <p role="alert" data-pane-state="unreadable" className="m-2 border border-status-err/60 p-2 text-xs text-status-err">
        The update settings in <code className="font-mono">{view.available.path}</code> could not be read (
        {view.available.error}). Fix or remove the file; nothing is checked until then.
      </p>
    )
  return (
    <>
      <WatchtowerBanner view={view} />
      {view.settings.checks_enabled ? <SettingsStrip view={view} /> : <ChecksOff settings={view.settings} />}
      <ServiceList services={view.services} />
      <Policies view={view} />
      <RecentRuns view={view} />
      <Registries view={view} />
    </>
  )
}

/** Checks are off: nothing has been asked of any registry. */
function ChecksOff({ settings }: { settings: UpdateSettings }) {
  const preview = useUpdates((s) => s.previewSettings)
  const blocked = useUntrusted()
  const [secs, setSecs] = useState(INTERVALS.some((i) => i.secs === settings.interval_secs) ? settings.interval_secs : 86_400)
  return (
    <div data-pane-state="checks-off" className="flex min-w-0 shrink-0 flex-col gap-1.5 border-b border-border p-2 text-xs">
      <p className="font-semibold">Update checks are off.</p>
      <p className="max-w-prose text-muted-foreground">
        With checks on, Depths asks each image&apos;s registry whether its tag points to a newer image, and shows the
        answer here and on each stack&apos;s page. A check only reads: it never pulls an image or restarts a container.
        Private registries use the credentials in Docker&apos;s config.json.
      </p>
      {blocked === null ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <label className="flex items-center gap-1">
            <span className="text-muted-foreground">Check</span>
            <select
              aria-label="Check interval"
              value={secs}
              onChange={(e) => setSecs(Number(e.target.value))}
              className={FIELD}
            >
              {INTERVALS.map((i) => (
                <option key={i.secs} value={i.secs}>
                  {i.label}
                </option>
              ))}
            </select>
          </label>
          <Button size="sm" data-action="turn-checks-on" onClick={() => preview(withChecks(settings, true, secs))}>
            Turn checks on
          </Button>
        </div>
      ) : (
        <p className="text-muted-foreground">Turn checks on from a trusted network.</p>
      )}
    </div>
  )
}

/** Checks are on: change the interval, turn them off, or open every setting. */
function SettingsStrip({ view }: { view: UpdatesView }) {
  const settings = view.settings
  const preview = useUpdates((s) => s.previewSettings)
  const blocked = useUntrusted()
  if (blocked !== null) return null
  const options = INTERVALS.some((i) => i.secs === settings.interval_secs)
    ? INTERVALS
    : [...INTERVALS, { secs: settings.interval_secs, label: intervalText(settings.interval_secs) }]
  return (
    <div className="flex min-w-0 shrink-0 flex-wrap items-center gap-1.5 border-b border-border px-2 py-1.5 text-xs">
      <label className="flex items-center gap-1">
        <span className="text-muted-foreground">Check</span>
        <select
          aria-label="Check interval"
          value={settings.interval_secs}
          onChange={(e) => preview(withChecks(settings, true, Number(e.target.value)))}
          className={FIELD}
        >
          {options.map((i) => (
            <option key={i.secs} value={i.secs}>
              {i.label}
            </option>
          ))}
        </select>
      </label>
      <Button variant="ghost" size="sm" data-action="turn-checks-off" onClick={() => preview(withChecks(settings, false))}>
        Turn checks off
      </Button>
      <SettingsSheet view={view} />
      <span data-field="cleanup-state" className="text-muted-foreground">
        {settings.cleanup ? 'Replaced images are removed' : 'Replaced images are kept'}
      </span>
      {isUtcZone(view.zone) && <ZoneNote zone={view.zone} />}
    </div>
  )
}

/** Select stacks with a newer image, then preview updating them. */
function BatchBar({ services }: { services: readonly ServiceUpdate[] }) {
  const selected = useUpdates((s) => s.selected)
  const setSelected = useUpdates((s) => s.setSelected)
  const previewBatch = useUpdates((s) => s.previewBatch)
  const running = useUpdates((s) => s.run !== null && !s.run.finished)
  const blocked = useUntrusted()
  const updatable = useMemo(() => updatableStacks(services), [services])
  if (blocked !== null || updatable.length === 0) return null
  const chosen = selected.filter((s) => updatable.includes(s))
  const all = chosen.length === updatable.length
  return (
    <div
      data-testid="updates-batch-bar"
      className="flex min-w-0 shrink-0 flex-wrap items-center gap-1.5 border-b border-border px-2 py-1.5 text-xs"
    >
      <label className="flex items-center gap-1">
        <input
          type="checkbox"
          data-action="select-all"
          checked={all}
          ref={(el) => {
            if (el) el.indeterminate = chosen.length > 0 && !all
          }}
          onChange={() => setSelected(all ? [] : updatable)}
        />
        <span>Every stack with an update ({String(updatable.length)})</span>
      </label>
      <span className="flex-1" />
      <Button size="sm" data-action="preview-batch" disabled={chosen.length === 0 || running} onClick={previewBatch}>
        {chosen.length === 0
          ? 'Select stacks to update'
          : `Update ${String(chosen.length)} ${chosen.length === 1 ? 'stack' : 'stacks'}`}
      </Button>
    </div>
  )
}

function ServiceList({ services }: { services: readonly ServiceUpdate[] }) {
  const stateFilter = useUpdates((s) => s.stateFilter)
  const stackFilter = useUpdates((s) => s.stackFilter)
  const setStateFilter = useUpdates((s) => s.setStateFilter)
  const setStackFilter = useUpdates((s) => s.setStackFilter)
  const counts = useMemo(() => countStates(services), [services])
  const stacks = useMemo(() => stacksOf(services), [services])
  const shown = useMemo(() => filterServices(services, stateFilter, stackFilter), [services, stateFilter, stackFilter])

  const scrollRef = useRef<HTMLDivElement>(null)
  // The app does not run the React Compiler; the virtualizer is read fresh every render.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: shown.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_ROW_PX,
    overscan: OVERSCAN,
  })
  const virtual = virtualizer.getVirtualItems()
  const first = virtual[0]
  const last = virtual[virtual.length - 1]
  const total = virtualizer.getTotalSize()

  return (
    <>
      <BatchBar services={services} />
      <div
        role="group"
        aria-label="Filter services"
        className="flex min-w-0 shrink-0 flex-wrap items-center gap-1 border-b border-border px-2 py-1.5"
      >
        <MagnifyingGlassIcon aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
        <select
          aria-label="State"
          value={stateFilter}
          onChange={(e) => setStateFilter(e.target.value as StateFilter)}
          className={`${FIELD} basis-36 flex-1 sm:flex-none`}
        >
          <option value="all">Every state ({String(services.length)})</option>
          {STATE_ORDER.filter((st) => (counts[st] ?? 0) > 0 || st === stateFilter).map((st) => (
            <option key={st} value={st}>
              {STATE_LABEL[st]} ({String(counts[st] ?? 0)})
            </option>
          ))}
        </select>
        <select
          aria-label="Stack"
          value={stackFilter}
          onChange={(e) => setStackFilter(e.target.value)}
          className={`${FIELD} basis-32 flex-1 sm:flex-none`}
        >
          <option value="">Every stack</option>
          {stacks.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        <span data-testid="updates-count" className="text-xs text-muted-foreground tabular-nums">
          {String(shown.length)} of {String(services.length)}
        </span>
      </div>

      <div
        ref={scrollRef}
        role="table"
        aria-label="Services and their image updates"
        aria-rowcount={shown.length + 1}
        data-testid="updates-scroller"
        className="min-h-48 min-w-0 flex-1 overflow-x-hidden overflow-y-auto"
      >
        <div role="rowgroup" className="sticky top-0 z-10 hidden border-b border-border bg-background md:block">
          <div role="row" aria-rowindex={1} className={`${ROW} font-semibold text-muted-foreground`}>
            <span role="columnheader">Service</span>
            <span role="columnheader">State</span>
            <span role="columnheader">Running</span>
            <span role="columnheader">Available</span>
            <span role="columnheader">Checked</span>
          </div>
        </div>
        {shown.length === 0 ? (
          <p role="status" data-pane-state="empty" className="p-3 text-xs text-muted-foreground">
            {services.length === 0 ? 'No services yet.' : 'No services match the filter.'}
          </p>
        ) : (
          <div role="rowgroup" style={{ paddingTop: first?.start ?? 0, paddingBottom: last ? total - last.end : 0 }}>
            {virtual.map((v) => {
              const s = shown[v.index]
              if (!s) return null
              return (
                <div key={v.key} data-index={v.index} ref={virtualizer.measureElement}>
                  <Row s={s} index={v.index} />
                </div>
              )
            })}
          </div>
        )}
      </div>
    </>
  )
}

function Row({ s, index }: { s: ServiceUpdate; index: number }) {
  const reason = reasonOf(s)
  const excluded = s.effective.excluded
  const available = availableText(s)
  const selected = useUpdates((st) => st.selected.includes(s.stack))
  const toggle = useUpdates((st) => st.toggleStack)
  const preview = useUpdates((st) => st.previewChange)
  const blocked = useUntrusted()
  const selectable = blocked === null && s.state === 'available'
  const target = s.available?.digest
  return (
    <div
      role="row"
      aria-rowindex={index + 2}
      data-update-row={`${s.stack}/${s.service}`}
      data-state={s.state}
      className={`${ROW} border-b border-border`}
    >
      <span className={LABEL}>Service</span>
      <span role="cell" className="min-w-0">
        {selectable && (
          <input
            type="checkbox"
            data-select-stack={s.stack}
            aria-label={`Select ${s.stack} for update`}
            className="mr-1 align-middle"
            checked={selected}
            onChange={() => toggle(s.stack)}
          />
        )}
        <Link to={`/stacks/${encodeURIComponent(s.stack)}`} className="hover:text-primary">
          {s.stack}
        </Link>
        <span className="text-muted-foreground"> / </span>
        {s.service}
        <span className="block truncate font-mono text-muted-foreground" title={s.reference}>
          {s.reference}
        </span>
      </span>
      <span className={LABEL}>State</span>
      <span role="cell" className="flex min-w-0 flex-col items-start gap-0.5">
        <UpdateStateChip state={s.state} />
      </span>
      <span className={LABEL}>Running</span>
      <span role="cell" className="min-w-0 font-mono break-all">
        {runningText(s)}
      </span>
      <span className={LABEL}>Available</span>
      <span role="cell" className="min-w-0 font-mono break-all">
        {available ?? '—'}
      </span>
      <span className={LABEL}>Checked</span>
      <span role="cell" className="tabular-nums text-muted-foreground">
        {s.last_checked ? <time dateTime={s.last_checked}>{formatTime(s.last_checked)}</time> : 'never'}
      </span>
      {(reason !== null || excluded !== null) && (
        <p data-field="reason" className="col-span-full min-w-0 break-words text-muted-foreground">
          {reason ?? (excluded ? exclusionText(excluded) : '')}
        </p>
      )}
      <p className="col-span-full flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
        <PolicyLine s={s} />
        {blocked === null && target && s.state === 'held' && (
          <Button
            variant="outline"
            size="sm"
            data-action="apply-held"
            onClick={() => preview({ kind: 'apply_held', data: { stack: s.stack, service: s.service, digest: target } })}
          >
            Apply anyway
          </Button>
        )}
        {blocked === null && target && s.state === 'rejected' && (
          <Button
            variant="outline"
            size="sm"
            data-action="retry"
            onClick={() => preview({ kind: 'retry', data: { stack: s.stack, service: s.service, digest: target } })}
          >
            Offer again
          </Button>
        )}
      </p>
      <LabelList s={s} />
      {isSelf(s) && (
        <div data-field="self-steps" className="col-span-full min-w-0 text-muted-foreground">
          Update Depths by hand, in the directory that holds its compose file:
          <pre className="mt-0.5 overflow-x-auto border border-border p-1 font-mono text-foreground">
            {SELF_UPDATE_STEPS.join('\n')}
          </pre>
        </div>
      )}
    </div>
  )
}

function Registries({ view }: { view: UpdatesView }) {
  return (
    <details data-testid="updates-registries" className="shrink-0 border-t border-border px-2 py-1.5 text-xs">
      <summary className="cursor-pointer text-muted-foreground">
        Registries ({String(view.registries.length)})
      </summary>
      {view.registries.length === 0 ? (
        <p className="mt-1 text-muted-foreground">No registry has been asked yet.</p>
      ) : (
        <ul className="mt-1 flex flex-col gap-0.5">
          {view.registries.map((r) => (
            <li key={r.host} data-registry={r.host} className="flex min-w-0 flex-wrap gap-x-2">
              <span className="font-mono">{r.host}</span>
              <span className={r.auth.kind === 'unsupported' ? 'text-status-err' : 'text-muted-foreground'}>
                {authText(r.auth)}
              </span>
              {r.backoff_until && (
                <span className="text-status-warn">
                  backed off until <time dateTime={r.backoff_until}>{formatTime(r.backoff_until)}</time>
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
      <p className="mt-1 max-w-prose text-muted-foreground">
        Depths reads registry credentials from the Docker config.json it is given and never shows or stores them.
        Credential helpers are not run.
      </p>
    </details>
  )
}

const CHANGE_TITLE: Record<UpdateChange['kind'], string> = {
  settings: 'Change update settings',
  policy: 'Change the update policy',
  ack_watchtower: 'Resume Auto updates',
  retry: 'Offer the image again',
  apply_held: 'Apply the held image',
}

/** Any update change (settings, a policy, a Watchtower answer, retry, apply held), before it is saved. */
function ChangeDialog() {
  const flow = useUpdates((s) => s.change)
  const confirm = useUpdates((s) => s.confirm)
  const cancel = useUpdates((s) => s.cancel)
  const open = flow.kind !== 'idle'
  const shown = flow.kind === 'preview' || flow.kind === 'verifying' || flow.kind === 'confirming' ? flow : null
  const starts = flow.kind !== 'idle' && flow.change.kind === 'apply_held'
  return (
    <Dialog open={open} onOpenChange={(o) => !o && cancel()}>
      <DialogContent
        data-testid="updates-settings-dialog"
        data-change={flow.kind === 'idle' ? undefined : flow.change.kind}
        className="max-h-[90dvh] overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>{flow.kind === 'idle' ? 'Change' : CHANGE_TITLE[flow.change.kind]}</DialogTitle>
          <DialogDescription>
            {starts
              ? 'Only this service is recreated. A new image that fails its health check is rolled back.'
              : 'Nothing changes until you confirm. The change is recorded in the audit log.'}
          </DialogDescription>
        </DialogHeader>
        {flow.kind === 'previewing' ? (
          <p role="status" className="text-xs text-muted-foreground">
            Preparing the change…
          </p>
        ) : flow.kind === 'refused' ? (
          <p role="alert" data-refusal className="text-xs text-status-err">
            {flow.reason}
          </p>
        ) : shown ? (
          <>
            <pre data-field="summary" className="border border-border p-2 font-mono text-xs whitespace-pre-wrap">
              {shown.preview.file_diff ?? ''}
            </pre>
            {shown.preview.steps.length > 0 && (
              <details className="text-xs">
                <summary className="cursor-pointer text-muted-foreground">
                  Steps ({String(shown.preview.steps.length)})
                </summary>
                <pre className="mt-0.5 overflow-x-auto border border-border p-1 font-mono">
                  {shown.preview.steps.map((st) => st.argv.join(' ')).join('\n')}
                </pre>
              </details>
            )}
            <StepUpNote stepUp={shown.stepUp} verifying={shown.kind === 'verifying'} note={shown.note} />
          </>
        ) : null}
        <DialogFooter>
          <Button variant="ghost" size="sm" onClick={cancel}>
            {flow.kind === 'refused' ? 'Close' : 'Cancel'}
          </Button>
          {shown && (
            <Button size="sm" data-action="confirm" disabled={shown.kind !== 'preview'} onClick={confirm}>
              {shown.kind === 'verifying' ? 'Waiting for passkey…' : starts ? 'Apply' : 'Confirm'}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
