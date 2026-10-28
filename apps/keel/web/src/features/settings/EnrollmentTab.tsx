/**
 * Settings → Enrollment (spec 060 US2): the snippet template and the instructions every enrolment
 * shows.
 *
 * The template is checked as it is typed, with the server's codes and order (`template.ts`, the
 * twin of keel_core), and rendered live with a dummy token. The instructions are Markdown, written
 * and previewed in tabs. Save goes through the confirm dialog with the version the edit started
 * from; if someone saved in between, the server refuses and a banner offers Reload.
 */
import { ArrowCounterClockwiseIcon, ArrowsClockwiseIcon, WarningIcon } from '@phosphor-icons/react'
import { Badge, Button } from '@companion/ui'
import { lazy, Suspense, useEffect, useId, useMemo, useState } from 'react'

import { RefusalText } from '@/features/teams/parts'
import { LoadState } from '@/ui/LoadState'
import { Tabs } from '@/ui/Tabs'

import { useSettings, type SettingsInfo } from './store'
import {
  byteLength,
  DEFAULT_TEMPLATE,
  errorText,
  MAX_BYTES,
  NO_RUNNER_ADDRESS,
  PLACEHOLDERS,
  render,
  validate,
} from './template'

const Markdown = lazy(() => import('./Markdown'))

const TEXTAREA =
  'block w-full min-w-0 resize-y border border-input bg-background p-2 font-mono text-xs outline-none focus-visible:border-accent-neon aria-[invalid=true]:border-status-err'
const SECTION = 'flex min-w-0 flex-col gap-2'
const H2 = 'font-display text-xs font-semibold tracking-wider uppercase'

/** The server's limit on the instructions (keel-server settings.rs). */
const MAX_INSTRUCTIONS_BYTES = 20 * 1024

/** A token-shaped stand-in for the preview: never a real token. */
const DUMMY_TOKEN = `keel-enroll-v1.${'0'.repeat(16)}EXAMPLE.SHA256:example`

export function ConflictBanner({ what, onReload }: { what: string; onReload: () => void }) {
  return (
    <div
      role="alert"
      data-conflict
      className="flex min-w-0 flex-wrap items-center gap-2 border border-status-warn/60 px-2 py-1.5 text-xs text-status-warn"
    >
      <WarningIcon aria-hidden className="size-3.5 shrink-0" />
      <span className="min-w-0 flex-1">
        Someone else saved changes to {what} since you loaded this page. Reload to see their version; your unsaved edits here will be replaced.
      </span>
      <Button variant="outline" size="xs" onClick={onReload}>
        <ArrowsClockwiseIcon aria-hidden />
        Reload
      </Button>
    </div>
  )
}

function Editor({ settings }: { settings: SettingsInfo }) {
  const save = useSettings((s) => s.saveSettings)
  const [template, setTemplate] = useState(settings.template)
  const [instructions, setInstructions] = useState(settings.instructions)
  const [view, setView] = useState<'write' | 'preview'>('write')
  const templateId = useId()
  const instructionsId = useId()
  // One expiry for the preview, fifteen minutes from when the editor opened.
  const [expiry] = useState(() => new Date(Date.now() + 15 * 60_000).toISOString().replace(/\.\d{3}Z$/, 'Z'))

  const error = validate(template)
  const bytes = byteLength(template)
  const preview = useMemo(
    () =>
      render(template, {
        t: DUMMY_TOKEN,
        n: 'web-01',
        p: 'base, web',
        w: 'ops',
        s: settings.runner_address ?? NO_RUNNER_ADDRESS,
        u: settings.public_url,
        e: expiry,
      }),
    [template, settings.runner_address, settings.public_url, expiry],
  )
  const usesRunner = settings.runner_address === null && /(^|[^%])(%%)*%s/.test(template)
  const changed = template !== settings.template || instructions !== settings.instructions
  const instructionsBytes = byteLength(instructions)
  const instructionsTooLong = instructionsBytes > MAX_INSTRUCTIONS_BYTES
  const blocked = error !== null || instructionsTooLong

  return (
    <form
      aria-label="Enrollment settings"
      className="flex min-w-0 flex-col gap-4"
      onSubmit={(e) => {
        e.preventDefault()
        if (!blocked && changed) save(template, instructions)
      }}
    >
      <section aria-labelledby={`${templateId}-h`} className={SECTION}>
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <h2 id={`${templateId}-h`} className={H2}>
            Snippet template
          </h2>
          {settings.is_default && template === settings.template && <Badge variant="outline">Default</Badge>}
        </div>
        <p className="text-xs text-muted-foreground">
          What the Enroll dialog tells people to run on a new host. It must include <code className="font-mono">%t</code>.
        </p>
        <div className="grid min-w-0 gap-3 lg:grid-cols-[minmax(0,1fr)_16rem]">
          <div className="flex min-w-0 flex-col gap-1">
            <label htmlFor={templateId} className="sr-only">
              Snippet template
            </label>
            <textarea
              id={templateId}
              rows={6}
              spellCheck={false}
              value={template}
              aria-invalid={error !== null}
              aria-describedby={`${templateId}-status`}
              className={TEXTAREA}
              onChange={(e) => {
                setTemplate(e.target.value)
              }}
            />
            <div id={`${templateId}-status`} className="flex min-w-0 flex-wrap items-start justify-between gap-2 text-xs">
              {error ? (
                <p data-template-error={error.code} className="min-w-0 text-status-err">
                  {errorText(error)}
                </p>
              ) : (
                <span />
              )}
              <span className={`tabular-nums ${bytes > MAX_BYTES ? 'text-status-err' : 'text-muted-foreground'}`}>
                {bytes} / {MAX_BYTES} bytes
              </span>
            </div>
            {usesRunner && (
              <p data-template-warning="no_runner_address" className="text-xs text-status-warn">
                %s renders as {NO_RUNNER_ADDRESS}: the runner listener is not configured on this server.
              </p>
            )}
          </div>
          <dl aria-label="Placeholders" className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] content-start gap-x-3 gap-y-0.5 text-xs">
            {PLACEHOLDERS.map((p) => (
              <div key={p.key} className="contents">
                <dt className="font-mono text-accent-neon">%{p.key}</dt>
                <dd className="text-muted-foreground">{p.meaning}</dd>
              </div>
            ))}
            <dt className="font-mono text-accent-neon">%%</dt>
            <dd className="text-muted-foreground">A literal %</dd>
          </dl>
        </div>
        <div className="flex min-w-0 flex-col gap-1">
          <h3 className="text-xs text-muted-foreground">Preview, with an example token</h3>
          <pre
            data-template-preview
            className="max-h-48 min-w-0 overflow-y-auto border border-border bg-muted/30 p-2 font-mono text-xs break-all whitespace-pre-wrap"
          >
            {preview}
          </pre>
        </div>
      </section>

      <section aria-labelledby={`${instructionsId}-h`} className={SECTION}>
        <h2 id={`${instructionsId}-h`} className={H2}>
          Instructions
        </h2>
        <p className="text-xs text-muted-foreground">
          Markdown shown under the snippet: firewall rules, who to ask, anything a person needs before running it.
        </p>
        <Tabs
          label="Instructions"
          items={[
            { value: 'write', label: 'Write' },
            { value: 'preview', label: 'Preview' },
          ]}
          value={view}
          onChange={setView}
        >
          {view === 'write' ? (
            <>
              <label htmlFor={instructionsId} className="sr-only">
                Instructions
              </label>
              <textarea
                id={instructionsId}
                rows={8}
                value={instructions}
                className={TEXTAREA}
                onChange={(e) => {
                  setInstructions(e.target.value)
                }}
              />
            </>
          ) : instructions.trim() === '' ? (
            <p data-pane-state="empty" className="text-xs text-muted-foreground">
              No instructions. The Enroll dialog shows only the snippet.
            </p>
          ) : (
            <div className="border border-border p-2">
              <Suspense fallback={<p className="text-xs text-muted-foreground">Loading preview…</p>}>
                <Markdown text={instructions} />
              </Suspense>
            </div>
          )}
        </Tabs>
        {instructionsTooLong && (
          <p data-instructions-error="instructions_too_long" className="text-xs text-status-err">
            The instructions are {instructionsBytes} bytes; the limit is {MAX_INSTRUCTIONS_BYTES}.
          </p>
        )}
      </section>

      <div className="flex min-w-0 flex-wrap items-center justify-end gap-2 border-t border-border pt-3">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={template === DEFAULT_TEMPLATE}
          onClick={() => {
            setTemplate(DEFAULT_TEMPLATE)
          }}
        >
          <ArrowCounterClockwiseIcon aria-hidden />
          Restore default
        </Button>
        <Button type="submit" size="sm" disabled={blocked || !changed}>
          Save
        </Button>
      </div>
    </form>
  )
}

export function EnrollmentTab() {
  const { value, refusal } = useSettings((s) => s.settings)
  const conflict = useSettings((s) => s.settingsConflict)
  const load = useSettings((s) => s.loadSettings)
  useEffect(() => {
    load()
  }, [load])
  return (
    <div className="flex min-w-0 flex-col gap-3">
      {conflict && <ConflictBanner what="these settings" onReload={load} />}
      <LoadState loading={value === null} error={refusal && <RefusalText refusal={refusal} />} label="Loading settings…">
        {/* A new version (saved here, or reloaded) starts the editor over from it. */}
        {value && <Editor key={value.version} settings={value} />}
      </LoadState>
    </div>
  )
}
