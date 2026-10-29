/**
 * `/admin/updates` (spec 062 US1, T041): every service's image update state, read-only.
 *
 * - Check now, the time zone and the next check in the header.
 * - While checks are off: what checking does, and a "Turn checks on" that goes through a preview.
 * - Filters by state and stack, then one virtualised scroller (the audit log's pattern), each
 *   service a grid row on wide screens and a card on narrow ones.
 * - The registries Depths has talked to, how it authenticates to each, and any back-off.
 *
 * Nothing here updates a container: batch updates and Auto come with later user stories.
 */
import { ArrowClockwiseIcon, FingerprintIcon, MagnifyingGlassIcon } from '@phosphor-icons/react'
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
import type { ServiceUpdate, UpdatesView, UpdateSettings } from '@/protocol/generated'

import {
  authText,
  availableText,
  countStates,
  exclusionText,
  filterServices,
  INTERVALS,
  intervalText,
  isSelf,
  reasonOf,
  runningText,
  SELF_UPDATE_STEPS,
  STATE_LABEL,
  STATE_ORDER,
  stacksOf,
  untilText,
  withChecks,
  type StateFilter,
} from './model'
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
      <SettingsDialog />
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
      {view.settings.checks_enabled ? <SettingsStrip settings={view.settings} /> : <ChecksOff settings={view.settings} />}
      <ServiceList services={view.services} />
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

/** Checks are on: change the interval or turn them off. */
function SettingsStrip({ settings }: { settings: UpdateSettings }) {
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

/** The settings change, before it is saved. */
function SettingsDialog() {
  const flow = useUpdates((s) => s.settings)
  const confirm = useUpdates((s) => s.confirm)
  const cancel = useUpdates((s) => s.cancel)
  const open = flow.kind !== 'idle'
  return (
    <Dialog open={open} onOpenChange={(o) => !o && cancel()}>
      <DialogContent data-testid="updates-settings-dialog">
        <DialogHeader>
          <DialogTitle>Change update settings</DialogTitle>
          <DialogDescription>Nothing changes until you confirm. The change is recorded in the audit log.</DialogDescription>
        </DialogHeader>
        {flow.kind === 'previewing' ? (
          <p role="status" className="text-xs text-muted-foreground">
            Preparing the change…
          </p>
        ) : flow.kind === 'refused' ? (
          <p role="alert" className="text-xs text-status-err">
            {flow.reason}
          </p>
        ) : flow.kind === 'preview' || flow.kind === 'verifying' || flow.kind === 'confirming' ? (
          <>
            <pre data-field="summary" className="border border-border p-2 font-mono text-xs whitespace-pre-wrap">
              {flow.preview.file_diff ?? ''}
            </pre>
            {flow.stepUp && (
              <p data-step-up className="flex items-start gap-1 text-xs text-muted-foreground">
                <FingerprintIcon aria-hidden className="mt-0.5 size-3 shrink-0" />
                <span>
                  {flow.kind === 'verifying'
                    ? 'Follow your browser’s passkey prompt…'
                    : 'Confirming asks for your passkey: this needs a fresh check that it’s you.'}
                </span>
              </p>
            )}
            {flow.note && (
              <p role="alert" data-refusal className="border border-status-warn/60 bg-status-warn/10 p-1.5 text-xs text-status-warn">
                {flow.note}
              </p>
            )}
          </>
        ) : null}
        <DialogFooter>
          <Button variant="ghost" size="sm" onClick={cancel}>
            {flow.kind === 'refused' ? 'Close' : 'Cancel'}
          </Button>
          {(flow.kind === 'preview' || flow.kind === 'verifying' || flow.kind === 'confirming') && (
            <Button size="sm" data-action="confirm" disabled={flow.kind !== 'preview'} onClick={confirm}>
              {flow.kind === 'verifying' ? 'Waiting for passkey…' : 'Confirm'}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
