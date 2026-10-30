/**
 * The owner's update card (spec 061 US2 to US5), on Settings → System.
 *
 * - Checks off (FR-004): says so and how to turn them on; no Check now.
 * - On: where the last check stands (not yet, checking, up to date, update available, older build,
 *   rebuilt, can't tell), when it ran and when the next one runs. A failed check keeps the last
 *   result and adds the failure's reason and the next attempt (FR-010). Check now runs one at once;
 *   within a minute of the last one it says when it may run again (FR-007).
 * - An image to move to: running and available side by side (version, revision, build date,
 *   digest), and the release notes or why there are none (FR-013). The navigation dot can be
 *   hidden for this digest (FR-014); the card stays.
 * - Update (FR-016 to FR-018): through the governance preview and its passkey step-up. A run in
 *   progress is linked instead; a failed one says so with a link, and Update is offered again.
 * - Without the prerequisites (FR-021): what is missing, the digest to pin and the steps to copy,
 *   and a link to the System workspace's setup below.
 */
import {
  ArrowCircleUpIcon,
  ArrowClockwiseIcon,
  ArrowSquareOutIcon,
  CheckCircleIcon,
  EyeSlashIcon,
  PlayIcon,
  WarningIcon,
} from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import type { ReactNode } from 'react'

import { Link } from '@/app/Link'
import { useSession } from '@/app/session-store'
import { RefusalText } from '@/features/teams/parts'
import type { AvailableImage, KeelBuild, UpdatePath, UpdateStatus } from '@/protocol/wire'
import { CopyBlock } from '@/ui/CopyBlock'
import { H2 } from '@/ui/form'
import { LoadState } from '@/ui/LoadState'

import { Row, ShortCopy, Time, Unknown } from './build-parts'
import { canUpdate, cardState, failedRunId, shortDigest, shortRevision, showDot, useUpdates, when, type CardState } from './updates'

const MUTED = 'text-xs text-muted-foreground'

/** Why a check failed, by `RegistryError::code()` (contracts/registry.md). */
const ERROR_TEXT: Record<string, string> = {
  auth_failed: 'the registry refused Keel’s credentials',
  not_found: 'the watched tag does not exist',
  rate_limited: 'the registry is rate-limiting Keel',
  timeout: 'the registry did not answer in time',
  network: 'the registry could not be reached',
  bad_response: 'the registry sent an answer Keel could not read',
  no_digest_header: 'the registry did not say which digest the tag points to',
}

/** Why Keel can't tell whether the watched image is newer. */
const CANT_TELL_TEXT: Record<string, string> = {
  no_image_for_platform: 'the watched tag has no image for this machine’s platform',
  missing_labels: 'the image carries no version or revision labels',
  no_revision_label: 'the image carries no revision label',
  running_unknown: 'this Keel knows neither its own digest nor its revision',
}

/** What is missing for the update to run as a Keel run (FR-021). */
const MISSING_TEXT: Record<string, string> = {
  no_system_workspace: 'There is no System workspace, so Keel cannot run its own update.',
  no_keel_host: 'No host in the System workspace is enrolled with the keel-server profile.',
  profile_not_allowed: 'The System workspace does not allow the keel-server profile.',
  job_missing_param: 'The bound repository’s keel-server job does not declare the keel_server_image_digest parameter.',
}

function Headline({ state, status }: { state: CardState; status: UpdateStatus }) {
  const line = (icon: ReactNode, text: ReactNode, tone = 'text-foreground') => (
    <p data-update-state={state} className={`flex items-start gap-1.5 text-sm ${tone}`}>
      {icon}
      <span className="min-w-0">{text}</span>
    </p>
  )
  const warn = <WarningIcon aria-hidden className="mt-0.5 size-4 shrink-0" />
  switch (state) {
    case 'off':
      return line(null, 'Update checks are off.', 'text-muted-foreground')
    case 'not_checked':
      return line(null, 'Not checked yet.', 'text-muted-foreground')
    case 'checking':
      return line(
        <ArrowClockwiseIcon aria-hidden className="mt-0.5 size-4 shrink-0 animate-spin motion-reduce:animate-none" />,
        'Checking for a newer Keel…',
        'text-muted-foreground',
      )
    case 'up_to_date':
      return line(<CheckCircleIcon aria-hidden className="mt-0.5 size-4 shrink-0" />, 'Keel is up to date.', 'text-status-ok')
    case 'update_available':
      return line(<ArrowCircleUpIcon aria-hidden className="mt-0.5 size-4 shrink-0" />, 'A newer Keel is available.', 'text-accent-neon')
    case 'older_build':
      return line(warn, 'The watched image is an older build than the one running. Updating to it goes back.', 'text-status-warn')
    case 'rebuilt':
      return line(
        warn,
        `The watched image is a rebuild of the running revision${status.running.revision ? ` ${shortRevision(status.running.revision)}` : ''}: same source, a different image.`,
        'text-status-warn',
      )
    case 'cant_tell': {
      const reason = status.cant_tell_reason
      return line(
        warn,
        `Keel can’t tell whether the watched image is newer: ${reason ? (CANT_TELL_TEXT[reason] ?? reason) : 'no reason given'}.`,
        'text-status-warn',
      )
    }
  }
}

function Off() {
  return (
    <p className={MUTED}>
      Keel contacts no registry until they are turned on. Set <code className="font-mono text-foreground">KEEL_UPDATE_CHECK=true</code> in
      Keel’s deployment configuration and restart it.
    </p>
  )
}

function Times({ status }: { status: UpdateStatus }) {
  return (
    <p className={`flex min-w-0 flex-col gap-0.5 sm:flex-row sm:flex-wrap sm:gap-x-3 sm:[&>*+*]:border-l sm:[&>*+*]:border-border sm:[&>*+*]:pl-3 ${MUTED}`}>
      {status.checked_at && (
        <span data-checked-at>
          Checked <Time at={status.checked_at} />
        </span>
      )}
      {status.next_check_at && (
        <span data-next-check>
          Next check <Time at={status.next_check_at} />
        </span>
      )}
      <span className="min-w-0 break-all">
        Watching <code className="font-mono">{status.image}</code>
      </span>
    </p>
  )
}

function Failure({ status }: { status: UpdateStatus }) {
  const error = status.last_error
  if (!error) return null
  return (
    <div role="alert" data-update-error={error.code} className="flex min-w-0 flex-col gap-0.5 border border-status-err/50 px-3 py-2 text-xs text-status-err">
      <span>
        The last check failed at <Time at={error.at} />: {ERROR_TEXT[error.code] ?? error.code}.
      </span>
      {status.next_check_at && (
        <span className="text-muted-foreground">
          Next attempt <Time at={status.next_check_at} />.
        </span>
      )}
      {status.outcome === null && <span className="text-muted-foreground">No check has succeeded yet.</span>}
    </div>
  )
}

function Build({ title, data, build }: { title: string; data: string; build: KeelBuild | AvailableImage }) {
  const version = build.version ?? ('display' in build ? build.display : null)
  return (
    <div data-build={data} className="flex min-w-0 flex-1 basis-56 flex-col gap-1 border border-border p-2">
      <h3 className="text-sm font-semibold">{title}</h3>
      <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-0.5 text-xs">
        <Row term="Version" data="version">
          {version !== null ? <span className="font-mono">{version}</span> : <Unknown text="untagged" />}
        </Row>
        <Row term="Revision" data="revision">
          {build.revision ? <ShortCopy label={`${data} revision`} short={shortRevision(build.revision)} value={build.revision} /> : <Unknown />}
        </Row>
        <Row term="Built" data="built">
          {build.built_at ? <Time at={build.built_at} /> : <Unknown />}
        </Row>
        <Row term="Digest" data="digest">
          {build.digest ? <ShortCopy label={`${data} digest`} short={shortDigest(build.digest)} value={build.digest} /> : <Unknown />}
        </Row>
      </dl>
    </div>
  )
}

function ReleaseNotes({ image }: { image: AvailableImage }) {
  if (image.release_notes) {
    return (
      <a
        href={image.release_notes}
        target="_blank"
        rel="noreferrer"
        data-release-notes
        className="inline-flex w-fit items-center gap-1 text-xs text-accent-neon hover:underline"
      >
        Release notes for {image.version ?? 'this image'}
        <ArrowSquareOutIcon aria-hidden className="size-3.5" />
      </a>
    )
  }
  const note = image.release_notes_note ?? (image.version === null ? 'No release notes for untagged builds.' : 'No release notes link for this image.')
  return (
    <p data-release-notes-note className={MUTED}>
      {note}
    </p>
  )
}

function Manual({ path }: { path: Extract<UpdatePath, { kind: 'manual' }> }) {
  return (
    <div data-update-manual={path.missing} className="flex min-w-0 flex-col gap-2 border-t border-border pt-3">
      <h3 className="text-sm font-semibold">Update by hand</h3>
      <p data-missing className="text-xs text-status-warn">
        {MISSING_TEXT[path.missing] ?? `Keel cannot run its own update (${path.missing}).`}
      </p>
      <CopyBlock label="Image to pin" value={path.digest_ref} data="digest-ref" />
      {path.steps.map((step, i) => (
        <CopyBlock key={i} label={`Step ${String(i + 1)}`} value={step} data={`step-${String(i + 1)}`} />
      ))}
      <a href="#system-workspace" className="inline-flex w-fit items-center gap-1 text-xs text-accent-neon hover:underline">
        Set up the System workspace so Keel can update itself
      </a>
    </div>
  )
}

function UpdateAction({ status }: { status: UpdateStatus }) {
  const run = useUpdates((s) => s.run)
  const update = useUpdates((s) => s.update)
  const available = status.available
  if (status.update_run !== null) {
    return (
      <p role="status" data-update-run="in_progress" className="flex min-w-0 flex-wrap items-center gap-2 text-xs">
        <PlayIcon aria-hidden className="size-3.5 text-accent-neon" />
        An update run is in progress.
        <Link to={`/runs/${status.update_run}`} className="text-accent-neon hover:underline">
          Open the update run
        </Link>
      </p>
    )
  }
  if (!canUpdate(status) || available === null) return null
  const failed = failedRunId(status, run)
  return (
    <div className="flex min-w-0 flex-col gap-2">
      {failed !== null && (
        <p role="alert" data-update-run="failed" className="flex min-w-0 flex-wrap items-center gap-2 text-xs text-status-err">
          The update failed; Keel is still running {status.running.display}.
          <Link to={`/runs/${failed}`} className="text-accent-neon hover:underline">
            Open the failed run
          </Link>
        </p>
      )}
      {status.update_path.kind === 'run' ? (
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <Button
            size="sm"
            // Moving to an older build or a rebuild is offered, not recommended (FR-011).
            variant={status.outcome === 'update_available' ? 'default' : 'outline'}
            onClick={() => {
              update(available.digest)
            }}
          >
            <ArrowCircleUpIcon aria-hidden />
            {failed !== null ? 'Try the update again' : 'Update Keel'}
          </Button>
          <span className={MUTED}>Runs keel-server in the System workspace. You’ll see a preview first.</span>
        </div>
      ) : (
        <Manual path={status.update_path} />
      )}
    </div>
  )
}

function Card({ status }: { status: UpdateStatus }) {
  const me = useSession((s) => s.me)
  const actionRefusal = useUpdates((s) => s.actionRefusal)
  const checkNow = useUpdates((s) => s.checkNow)
  const dismissDot = useUpdates((s) => s.dismissDot)
  const state = cardState(status)
  const available = status.available
  const compare = available !== null && status.outcome !== null && status.outcome !== 'up_to_date'
  const tooSoon = actionRefusal?.kind === 'error' && actionRefusal.code === 'update_check_too_soon'
  return (
    <>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <h2 className={`${H2} flex items-center gap-1.5`}>
          <ArrowCircleUpIcon aria-hidden className="size-3.5 text-accent-neon" />
          Updates
        </h2>
        <span className="flex-1" />
        {status.enabled && (
          <Button variant="outline" size="xs" disabled={state === 'checking'} onClick={checkNow}>
            <ArrowClockwiseIcon aria-hidden />
            Check now
          </Button>
        )}
        {showDot(me, status) && available && (
          <Button
            variant="ghost"
            size="xs"
            className="text-muted-foreground hover:text-foreground"
            onClick={() => {
              dismissDot(available.digest)
            }}
          >
            <EyeSlashIcon aria-hidden />
            Hide the navigation dot
          </Button>
        )}
      </div>
      {tooSoon ? (
        <p role="status" data-check-too-soon className="text-xs text-status-warn">
          Check now runs at most once a minute. Try again at{' '}
          <time dateTime={actionRefusal.message}>{/^\d{4}-/.test(actionRefusal.message) ? when(actionRefusal.message) : actionRefusal.message}</time>.
        </p>
      ) : (
        actionRefusal && <RefusalText refusal={actionRefusal} />
      )}
      <div aria-live="polite" className="min-w-0">
        <Headline state={state} status={status} />
      </div>
      {state === 'off' ? <Off /> : <Times status={status} />}
      <Failure status={status} />
      {!status.digest_compare && status.enabled && status.outcome !== null && (
        <p data-compared-by="revision" className={MUTED}>
          This Keel doesn’t know its own image digest, so the builds were compared by revision.
        </p>
      )}
      {compare && (
        <>
          <div className="flex min-w-0 flex-wrap gap-2">
            <Build title="Running" data="running" build={status.running} />
            <Build title="Available" data="available" build={available} />
          </div>
          <ReleaseNotes image={available} />
        </>
      )}
      <UpdateAction status={status} />
    </>
  )
}

export function UpdateCard() {
  const status = useUpdates((s) => s.status)
  const refusal = useUpdates((s) => s.refusal)
  return (
    <section
      aria-label="Updates"
      data-update-card={status ? cardState(status) : 'loading'}
      className="flex min-w-0 flex-col gap-2 border border-border p-3"
    >
      <LoadState loading={status === null} error={refusal && <RefusalText refusal={refusal} />} label="Reading the update status…">
        {status && <Card status={status} />}
      </LoadState>
    </section>
  )
}
