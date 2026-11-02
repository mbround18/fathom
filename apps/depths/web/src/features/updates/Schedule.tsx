/**
 * US3–US5 on the Updates page (T062, T071, T077):
 *
 * - The settings sheet: interval, maintenance window, health timeout, cleanup, label-only and scope.
 * - Each stack's policy (Off, Notify, Auto) and its own window, with who set it; Auto policies whose
 *   author is no longer an administrator are paused until someone saves them again.
 * - The Watchtower banner: Auto is paused while a Watchtower container runs, until an admin says
 *   it is stopped or only monitors.
 * - Recent runs: what each did per stack, the output of a failed rollback, and the old images removed.
 *
 * Every change goes through a preview and a passkey (`useUpdates.previewChange`).
 */
import { ClockIcon, GearSixIcon, WarningIcon } from '@phosphor-icons/react'
import { Button, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@companion/ui'
import { useEffect, useId, useMemo, useState } from 'react'

import { formatTime } from '@/features/stacks/model'
import type {
  AuditEntry,
  MaintenanceWindow,
  StackUpdatePolicy,
  UpdatePolicy,
  UpdateRun,
  UpdateSettings,
  UpdatesView,
} from '@/protocol/generated'

import {
  HEALTH_TIMEOUT,
  INTERVALS,
  intervalText,
  isUtcZone,
  outcomeText,
  outcomeTone,
  POLICY_HELP,
  POLICY_LABEL,
  runEntries,
  settingsProblem,
  stacksOf,
  triggerText,
  WEEKDAYS,
  windowProblem,
  windowText,
} from './model'
import { useUpdates } from './store'

const FILTER_FIELD = 'h-7 min-w-0 border border-input bg-background px-1.5 text-xs outline-none focus:border-ring'
const POLICIES: readonly UpdatePolicy[] = ['off', 'notify', 'auto']

// --- Window editor ------------------------------------------------------------------------------

export function WindowEditor({
  value,
  onChange,
  label,
  disabled = false,
}: {
  value: MaintenanceWindow
  onChange: (w: MaintenanceWindow) => void
  label: string
  disabled?: boolean
}) {
  const id = useId()
  const toggle = (day: MaintenanceWindow['days'][number]) =>
    onChange({
      ...value,
      days: value.days.includes(day)
        ? value.days.filter((d) => d !== day)
        : WEEKDAYS.map((w) => w.day).filter((d) => d === day || value.days.includes(d)),
    })
  const problem = windowProblem(value)
  return (
    <fieldset data-field="window" disabled={disabled} className="flex min-w-0 flex-col gap-1 text-xs">
      <legend className="mb-1 text-muted-foreground">{label}</legend>
      <div className="flex flex-wrap gap-1" role="group" aria-label="Days">
        {WEEKDAYS.map((d) => (
          <label
            key={d.day}
            className="flex h-7 cursor-pointer items-center gap-1 border border-border px-1.5 has-[:checked]:border-primary has-[:checked]:text-primary"
          >
            <input type="checkbox" data-day={d.day} checked={value.days.includes(d.day)} onChange={() => toggle(d.day)} />
            {d.short}
          </label>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <label className="flex items-center gap-1" htmlFor={`${id}-start`}>
          <span className="text-muted-foreground">From</span>
          <input
            id={`${id}-start`}
            type="time"
            data-field="window-start"
            value={value.start}
            onChange={(e) => onChange({ ...value, start: e.target.value })}
            className={FILTER_FIELD}
          />
        </label>
        <label className="flex items-center gap-1" htmlFor={`${id}-end`}>
          <span className="text-muted-foreground">to</span>
          <input
            id={`${id}-end`}
            type="time"
            data-field="window-end"
            value={value.end}
            onChange={(e) => onChange({ ...value, end: e.target.value })}
            className={FILTER_FIELD}
          />
        </label>
        {problem === null && value.end < value.start && <span className="text-muted-foreground">ends the next day</span>}
      </div>
      {problem && (
        <p role="alert" data-field="window-problem" className="text-status-err">
          {problem}
        </p>
      )}
    </fieldset>
  )
}

/** Windows are in the server's zone; with no /etc/localtime that is UTC. */
export function ZoneNote({ zone }: { zone: string }) {
  if (!isUtcZone(zone)) return <p className="text-xs text-muted-foreground">Times are in the server&apos;s zone, {zone}.</p>
  return (
    <p data-field="zone-utc" className="text-xs text-status-warn">
      Windows are in UTC: mount /etc/localtime to use host time.
    </p>
  )
}

// --- Settings sheet -----------------------------------------------------------------------------

export function SettingsSheet({ view }: { view: UpdatesView }) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button variant="ghost" size="sm" data-action="edit-settings" onClick={() => setOpen(true)}>
        <GearSixIcon aria-hidden />
        Settings
      </Button>
      {open && <SettingsForm view={view} onClose={() => setOpen(false)} />}
    </>
  )
}

function SettingsForm({ view, onClose }: { view: UpdatesView; onClose: () => void }) {
  const preview = useUpdates((s) => s.previewSettings)
  const [draft, setDraft] = useState<UpdateSettings>(view.settings)
  const problem = settingsProblem(draft)
  const changed = JSON.stringify(draft) !== JSON.stringify(view.settings)
  const intervals = INTERVALS.some((i) => i.secs === draft.interval_secs)
    ? INTERVALS
    : [...INTERVALS, { secs: draft.interval_secs, label: intervalText(draft.interval_secs) }]
  const set = (over: Partial<UpdateSettings>) => setDraft((d) => ({ ...d, ...over }))
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent data-testid="updates-settings-sheet" className="max-h-[90dvh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Update settings</DialogTitle>
          <DialogDescription>For the whole host. The next step shows the change before anything is saved.</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-3 text-xs">
          <label className="flex flex-wrap items-center gap-1.5">
            <span className="text-muted-foreground">Check</span>
            <select
              aria-label="Check interval"
              value={draft.interval_secs}
              onChange={(e) => set({ interval_secs: Number(e.target.value) })}
              className={FILTER_FIELD}
            >
              {intervals.map((i) => (
                <option key={i.secs} value={i.secs}>
                  {i.label}
                </option>
              ))}
            </select>
          </label>
          <div className="flex flex-col gap-1">
            <WindowEditor label="Maintenance window: when Auto stacks are updated" value={draft.window} onChange={(window) => set({ window })} />
            <ZoneNote zone={view.zone} />
          </div>
          <label className="flex flex-wrap items-center gap-1.5">
            <span className="text-muted-foreground">Health timeout</span>
            <input
              type="number"
              data-field="health-timeout"
              min={HEALTH_TIMEOUT.min}
              max={HEALTH_TIMEOUT.max}
              step={30}
              value={draft.health_timeout_secs}
              onChange={(e) => set({ health_timeout_secs: Number(e.target.value) })}
              className={`${FILTER_FIELD} w-20 tabular-nums`}
            />
            <span className="text-muted-foreground">seconds for a new container to become healthy before it is rolled back</span>
          </label>
          <label className="flex items-start gap-1.5">
            <input
              type="checkbox"
              data-field="cleanup"
              className="mt-0.5"
              checked={draft.cleanup}
              onChange={(e) => set({ cleanup: e.target.checked })}
            />
            <span>
              Remove replaced images after an update
              <span className="block text-muted-foreground">
                The image a service ran before is deleted once the new one is healthy. An image another container still uses is
                kept. Without this, old images stay until you prune them.
              </span>
            </span>
          </label>
          <label className="flex items-start gap-1.5">
            <input
              type="checkbox"
              data-field="label-only"
              className="mt-0.5"
              checked={draft.label_only}
              onChange={(e) => set({ label_only: e.target.checked })}
            />
            <span>
              Only services with the Watchtower enable label
              <span className="block text-muted-foreground">Like WATCHTOWER_LABEL_ENABLE: every other service is left alone.</span>
            </span>
          </label>
          <label className="flex flex-wrap items-center gap-1.5">
            <span className="text-muted-foreground">Watchtower scope</span>
            <input
              type="text"
              data-field="scope"
              placeholder="none"
              value={draft.scope ?? ''}
              onChange={(e) => {
                const v = e.target.value.trim()
                setDraft((d) => {
                  const next = { ...d }
                  if (v === '') delete next.scope
                  else next.scope = v
                  return next
                })
              }}
              className={`${FILTER_FIELD} w-32`}
            />
            <span className="text-muted-foreground">only services whose scope label matches</span>
          </label>
          {problem && (
            <p role="alert" data-field="settings-problem" className="text-status-err">
              {problem}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button
            size="sm"
            data-action="preview-settings"
            disabled={problem !== null || !changed}
            onClick={() => {
              preview(draft)
              onClose()
            }}
          >
            Review change
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

// --- Policies -----------------------------------------------------------------------------------

export function Policies({ view }: { view: UpdatesView }) {
  const stacks = useMemo(
    () => [...new Set([...stacksOf(view.services), ...Object.keys(view.policies)])].sort(),
    [view.services, view.policies],
  )
  const [editing, setEditing] = useState<string | null>(null)
  if (stacks.length === 0) return null
  return (
    <details data-testid="updates-policies" className="shrink-0 border-t border-border px-2 py-1.5 text-xs" open={view.paused_policies.length > 0 || undefined}>
      <summary className="cursor-pointer text-muted-foreground">
        Stack policies ({String(stacks.length)}){view.paused_policies.length > 0 && <span className="text-status-warn"> · {String(view.paused_policies.length)} paused</span>}
      </summary>
      <p className="mt-1 max-w-prose text-muted-foreground">
        A stack nobody has changed is Notify. Auto installs new images in the maintenance window ({windowText(view.settings.window)}
        {view.next_window ? `, next ${formatTime(view.next_window)}` : ''}), on behalf of the administrator who set it.
      </p>
      <ul className="mt-1 flex max-h-[30dvh] flex-col overflow-y-auto">
        {stacks.map((stack) => (
          <PolicyRow
            key={stack}
            stack={stack}
            policy={view.policies[stack] ?? null}
            paused={view.paused_policies.includes(stack)}
            onEdit={() => setEditing(stack)}
          />
        ))}
      </ul>
      {editing !== null && (
        <PolicySheet
          stack={editing}
          current={view.policies[editing] ?? null}
          hostWindow={view.settings.window}
          zone={view.zone}
          onClose={() => setEditing(null)}
        />
      )}
    </details>
  )
}

function PolicyRow({
  stack,
  policy,
  paused,
  onEdit,
}: {
  stack: string
  policy: StackUpdatePolicy | null
  paused: boolean
  onEdit: () => void
}) {
  const preview = useUpdates((s) => s.previewChange)
  const p = policy?.policy ?? 'notify'
  return (
    <li data-policy-row={stack} data-policy={p} className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 border-b border-border py-1">
      <span className="min-w-24 font-semibold">{stack}</span>
      <span data-field="policy" className={p === 'auto' ? 'text-primary' : ''}>
        {POLICY_LABEL[p]}
      </span>
      {policy?.window && (
        <span data-field="own-window" className="text-muted-foreground">
          <ClockIcon aria-hidden className="mr-0.5 inline size-3" />
          {windowText(policy.window)}
        </span>
      )}
      <span className="text-muted-foreground">
        {policy ? (
          <>
            set by {policy.set_by.username} <time dateTime={policy.set_at}>{formatTime(policy.set_at)}</time>
          </>
        ) : (
          'default'
        )}
      </span>
      {paused && (
        <span data-field="paused" className="text-status-warn">
          paused: author no longer an administrator
        </span>
      )}
      <span className="flex-1" />
      {paused && policy && (
        <Button
          variant="outline"
          size="sm"
          data-action="resume-policy"
          onClick={() => preview({ kind: 'policy', data: { stack, policy: policy.policy, ...(policy.window ? { window: policy.window } : {}) } })}
        >
          Resume as me
        </Button>
      )}
      <Button variant="ghost" size="sm" data-action="edit-policy" aria-label={`Change the update policy of ${stack}`} onClick={onEdit}>
        Change
      </Button>
    </li>
  )
}

function PolicySheet({
  stack,
  current,
  hostWindow,
  zone,
  onClose,
}: {
  stack: string
  current: StackUpdatePolicy | null
  hostWindow: MaintenanceWindow
  zone: string
  onClose: () => void
}) {
  const preview = useUpdates((s) => s.previewChange)
  const [policy, setPolicy] = useState<UpdatePolicy>(current?.policy ?? 'notify')
  const [own, setOwn] = useState(current?.window != null)
  const [window, setWindow] = useState<MaintenanceWindow>(current?.window ?? hostWindow)
  const problem = own && policy === 'auto' ? windowProblem(window) : null
  const name = useId()
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent data-testid="updates-policy-sheet" className="max-h-[90dvh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Update policy of {stack}</DialogTitle>
          <DialogDescription>The next step shows the change before anything is saved.</DialogDescription>
        </DialogHeader>
        <fieldset className="flex flex-col gap-1.5 text-xs">
          <legend className="sr-only">Policy</legend>
          {POLICIES.map((p) => (
            <label
              key={p}
              aria-label={POLICY_LABEL[p]}
              className="flex cursor-pointer items-start gap-1.5 border border-border p-1.5 has-[:checked]:border-primary"
            >
              <input type="radio" name={name} value={p} data-policy-option={p} className="mt-0.5" checked={policy === p} onChange={() => setPolicy(p)} />
              <span>
                <span className="font-semibold">{POLICY_LABEL[p]}</span>
                <span className="block text-muted-foreground">{POLICY_HELP[p]}</span>
              </span>
            </label>
          ))}
        </fieldset>
        {policy === 'auto' && (
          <div className="flex flex-col gap-1.5 text-xs">
            <label className="flex items-center gap-1.5">
              <input type="checkbox" data-field="own-window" checked={own} onChange={(e) => setOwn(e.target.checked)} />
              <span>Use its own window instead of the host window ({windowText(hostWindow)})</span>
            </label>
            {own && <WindowEditor label={`Window for ${stack}`} value={window} onChange={setWindow} />}
            <ZoneNote zone={zone} />
          </div>
        )}
        <DialogFooter>
          <Button variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button
            size="sm"
            data-action="preview-policy"
            disabled={problem !== null}
            onClick={() => {
              preview({ kind: 'policy', data: { stack, policy, ...(policy === 'auto' && own ? { window } : {}) } })
              onClose()
            }}
          >
            Review change
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

// --- Watchtower ---------------------------------------------------------------------------------

export function WatchtowerBanner({ view }: { view: UpdatesView }) {
  const preview = useUpdates((s) => s.previewChange)
  const open = view.watchtower.filter((w) => w.acknowledged === null)
  const paused = view.auto_paused
  if (open.length === 0 && paused === null) return null
  if (open.length === 0) {
    return (
      <p role="status" data-testid="updates-paused" className="flex shrink-0 items-start gap-1 border-b border-status-warn/60 bg-status-warn/10 px-2 py-1.5 text-xs text-status-warn">
        <WarningIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
        {paused?.kind === 'disabled'
          ? 'Auto updates are turned off on this server; Notify still shows new images.'
          : paused?.kind === 'settings_unreadable'
            ? 'Auto updates are paused: the update settings could not be read.'
            : 'Auto updates are paused.'}
      </p>
    )
  }
  return (
    <div role="alert" data-testid="updates-watchtower" className="flex shrink-0 flex-col gap-1 border-b border-status-warn/60 bg-status-warn/10 px-2 py-1.5 text-xs">
      <p className="flex items-start gap-1 text-status-warn">
        <WarningIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
        <span>
          Auto updates are paused: Watchtower is running ({open.map((w) => w.name).join(', ')}). Two updaters on one host
          replace each other&apos;s containers. Stop it, or tell Depths it only monitors.
        </span>
      </p>
      <ul className="flex flex-col gap-1">
        {open.map((w) => (
          <li key={w.id} data-watchtower={w.name} className="flex min-w-0 flex-wrap items-center gap-1.5">
            <span className="font-semibold">{w.name}</span>
            <span className="min-w-0 truncate font-mono text-muted-foreground">{w.image}</span>
            <span className="flex-1" />
            <Button
              variant="outline"
              size="sm"
              data-action="ack-stopped"
              onClick={() => preview({ kind: 'ack_watchtower', data: { container_id: w.id, answer: 'stopped' } })}
            >
              I stopped it
            </Button>
            <Button
              variant="outline"
              size="sm"
              data-action="ack-monitor-only"
              onClick={() => preview({ kind: 'ack_watchtower', data: { container_id: w.id, answer: 'monitor_only' } })}
            >
              It only monitors
            </Button>
          </li>
        ))}
      </ul>
    </div>
  )
}

// --- Recent runs --------------------------------------------------------------------------------

export function RecentRuns({ view }: { view: UpdatesView }) {
  const runAudit = useUpdates((s) => s.runAudit)
  const load = useUpdates((s) => s.loadRunAudit)
  const runs = view.recent_runs
  useEffect(() => {
    if (runs.length > 0) load()
  }, [runs.length, load])
  const entries = useMemo(() => [...(runAudit.image_remove ?? []), ...(runAudit.image_rollback ?? [])], [runAudit])
  if (runs.length === 0) return null
  return (
    <details data-testid="updates-runs" className="shrink-0 border-t border-border px-2 py-1.5 text-xs">
      <summary className="cursor-pointer text-muted-foreground">Recent runs ({String(runs.length)})</summary>
      <ul className="mt-1 flex max-h-[30dvh] flex-col gap-1.5 overflow-y-auto">
        {runs.map((r) => (
          <RunItem key={r.run_id} run={r} entries={entries} />
        ))}
      </ul>
    </details>
  )
}

function RunItem({ run, entries }: { run: UpdateRun; entries: readonly AuditEntry[] }) {
  return (
    <li data-run={run.run_id} className="flex min-w-0 flex-col gap-0.5 border-b border-border pb-1">
      <p className="text-muted-foreground">
        <time dateTime={run.started_at}>{formatTime(run.started_at)}</time>, {triggerText(run.trigger)}
        {run.finished_at === null && ', still running'}
      </p>
      <ul className="flex flex-col gap-0.5">
        {run.stacks.map((s) => {
          const mine = runEntries(run, s.stack, entries)
          const failed = mine.find((e) => e.action === 'image_rollback' && e.outcome.kind === 'failed')
          const removed = mine.filter((e) => e.action === 'image_remove')
          return (
            <li key={s.stack} data-run-stack={s.stack} data-outcome={s.outcome?.kind ?? 'pending'} className="flex min-w-0 flex-col gap-0.5">
              <span className="flex min-w-0 flex-wrap items-center gap-1.5">
                <span className="font-semibold">{s.stack}</span>
                {s.outcome ? (
                  <span className={`inline-flex h-5 items-center border px-1.5 ${outcomeTone(s.outcome)}`}>{outcomeText(s.outcome)}</span>
                ) : (
                  <span className="text-muted-foreground">{s.phase}</span>
                )}
                {s.services.length > 0 && <span className="text-muted-foreground">{s.services.map((c) => c.service).join(', ')}</span>}
              </span>
              {failed?.outcome.kind === 'failed' && failed.outcome.stderr_tail !== '' && (
                <pre
                  data-field="run-output"
                  className={`overflow-x-auto border p-1 font-mono whitespace-pre-wrap break-all ${s.outcome?.kind === 'rollback_failed' ? 'border-status-err/60 text-status-err' : 'border-border text-muted-foreground'}`}
                >
                  {failed.outcome.stderr_tail}
                </pre>
              )}
              {removed.map((e) => (
                <span key={`${e.at}-${e.diff_summary ?? ''}`} data-field="removed" className="min-w-0 font-mono break-all text-muted-foreground">
                  {e.diff_summary}
                </span>
              ))}
            </li>
          )
        })}
      </ul>
    </li>
  )
}
