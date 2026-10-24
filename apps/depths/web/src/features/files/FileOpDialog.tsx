/**
 * The confirm dialog for every change to files (spec 053, US2–US5): what the server says the
 * change will do, before it does it. Mounted once in `FilesTab`; `useFileOps` drives it.
 *
 * - The server's summary verbatim (the full path, old → new for a rename), its diff for a save or
 *   a restore, the entry count for a folder delete ("10,000+" when capped), and for an upload the
 *   folder and every file, replacements marked.
 * - When a service using the mount is running, a warning. A save offers **Save** and, when the
 *   person may restart the stack, **Save and restart <service>**, which runs the ordinary
 *   `PreviewOp{service_restart}` → Confirm after the save lands. An upload offers to stop the
 *   service first through the ordinary `PreviewOp{service_stop}`, then previews again.
 * - Escape and Cancel close it without sending anything. Radix returns focus to the control that
 *   opened it.
 */
import { WarningIcon } from '@phosphor-icons/react'
import { Button, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@companion/ui'
import { useEffect, useRef } from 'react'

import { useSession } from '@/app/session-store'
import { formatBytes } from '@/features/stacks/metrics'
import { useOps } from '@/features/stacks/ops-store'
import { DiffText } from '@/features/stacks/PreviewDialog'
import type { FileOp, FilePreview } from '@/protocol/generated'

import { isSaveConflict, useFileOps, type FileOpRefusal } from './fileops-store'
import { mayRestart } from './model'
import { refusalText, relist } from './store'

const TITLE: Record<FileOp['kind'], string> = {
  save: 'Save file',
  create: 'Create',
  rename: 'Rename or move',
  delete: 'Delete',
  restore: 'Restore a version',
  upload: 'Upload files',
}

const CONFIRM: Record<FileOp['kind'], string> = {
  save: 'Save',
  create: 'Create',
  rename: 'Rename',
  delete: 'Delete',
  restore: 'Restore',
  upload: 'Upload',
}

const VOID: Record<string, string> = {
  unknown: 'This preview is no longer known to the server. Preview it again.',
  used: 'This preview was already used. Preview it again.',
  expired: 'This preview expired (they last five minutes). Preview it again.',
  stack_changed: 'The stack changed since this preview. Preview it again.',
}

function fileOpRefusalText(r: FileOpRefusal): string {
  if (r.kind === 'void') return VOID[r.reason] ?? 'This preview is void. Preview it again.'
  if (r.kind === 'lost') return 'The connection dropped before the server answered; the change may or may not have landed. Check the folder, then try again.'
  return refusalText(r)
}

const count = (n: number): string => n.toLocaleString('en-US')

function Running({ services }: { services: readonly string[] }) {
  if (services.length === 0) return null
  const names = services.join(', ')
  return (
    <p data-running={services.join(' ')} role="note" className="flex items-start gap-1 border border-status-warn/60 bg-status-warn/10 p-1.5 text-xs text-status-warn">
      <WarningIcon aria-hidden className="mt-0.5 size-3 shrink-0" />
      <span>
        {names} {services.length === 1 ? 'is' : 'are'} running and may overwrite this change, or not notice it, until{' '}
        {services.length === 1 ? 'it restarts' : 'they restart'}.
      </span>
    </p>
  )
}

function Body({ preview, op }: { preview: FilePreview; op: FileOp }) {
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <p data-field="summary" className="font-mono text-xs break-all">
        {preview.summary}
      </p>
      {op.kind === 'delete' && preview.entries !== null && (
        <p data-field="entries" className="text-xs">
          With everything inside it:{' '}
          <span className="font-semibold">
            {preview.entries_capped ? `${count(preview.entries)}+` : count(preview.entries)} {preview.entries === 1 ? 'entry' : 'entries'}
          </span>
          .
        </p>
      )}
      {op.kind === 'delete' && (
        <p data-destructive role="note" className="text-xs text-status-err">
          A deleted file's content is kept in its history; folders are not.
        </p>
      )}
      {preview.diff !== null && (
        <section aria-label="Changes" className="flex min-w-0 flex-col gap-1">
          <h3 className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">Changes</h3>
          {preview.diff.trim() === '' ? <p className="text-xs text-muted-foreground">No changes.</p> : <DiffText diff={preview.diff} />}
        </section>
      )}
      {preview.uploads && preview.uploads.length > 0 && (
        <ul data-uploads aria-label="Files to upload" className="min-w-0 border border-border bg-background/40 px-2 text-xs">
          {preview.uploads.map((u) => (
            <li
              key={u.name}
              data-upload={u.name}
              data-replaces={u.exists || undefined}
              className="flex min-w-0 flex-wrap items-baseline gap-x-2 border-t border-border py-1 first:border-t-0"
            >
              <span className="min-w-0 font-mono break-all">{u.name}</span>
              <span className="text-muted-foreground tabular-nums">{formatBytes(u.size)}</span>
              {u.exists && <span className="text-status-warn">replaces the existing file</span>}
            </li>
          ))}
        </ul>
      )}
      <Running services={preview.running} />
      <p className="text-xs text-muted-foreground">
        Valid once, until <span data-field="expires">{preview.expires_at}</span>.
      </p>
    </div>
  )
}

export function FileOpDialog() {
  const flow = useFileOps((s) => s.flow)
  const confirm = useFileOps((s) => s.confirm)
  const dismiss = useFileOps((s) => s.dismiss)
  const retry = useFileOps((s) => s.retry)
  const lifecycleOpen = useOps((s) => s.flow !== null)
  const requestOp = useOps((s) => s.requestPreview)
  const me = useSession((s) => s.me)

  // "Stop first": the lifecycle dialog takes over, and this one previews again when it closes.
  const stopping = useRef(false)
  useEffect(() => {
    if (!lifecycleOpen && stopping.current) {
      stopping.current = false
      if (useFileOps.getState().flow) retry()
    }
  }, [lifecycleOpen, retry])

  if (!flow || isSaveConflict(flow) || lifecycleOpen) return null
  const { op, preview, phase, refusal, stack } = flow
  // A partial change (a delete stopped part way) changed the folder: list it again on close.
  const close = () => {
    if (refusal?.kind === 'partial') relist()
    dismiss()
  }
  const busy = phase === 'previewing' || phase === 'confirming'
  const ready = phase === 'ready' && preview !== null
  const running = preview?.running ?? []
  const restart = op.kind === 'save' && mayRestart(me, stack) ? running : []
  const stopFirst = op.kind === 'upload' && mayRestart(me, stack) ? running : []
  const canRetry = refusal?.kind === 'void' || refusal?.kind === 'lost'
  const destructive = op.kind === 'delete'

  return (
    <Dialog open onOpenChange={(o) => !o && close()}>
      <DialogContent data-file-op={op.kind} className="max-h-[90dvh] overflow-y-auto sm:max-w-2xl" aria-describedby={undefined}>
        <DialogHeader>
          <DialogTitle className="font-display tracking-wider break-words">
            {TITLE[op.kind]} in {stack}
          </DialogTitle>
          <DialogDescription>Nothing changes until you confirm.</DialogDescription>
        </DialogHeader>
        {preview ? (
          <Body preview={preview} op={op} />
        ) : phase === 'previewing' ? (
          <p role="status" data-pane-state="loading" className="text-xs text-muted-foreground">
            Building the preview…
          </p>
        ) : null}
        {refusal && (
          <p data-refusal={refusal.kind} role="alert" className="border border-status-warn/60 bg-status-warn/10 p-1.5 text-xs break-words text-status-warn">
            {fileOpRefusalText(refusal)}
          </p>
        )}
        <DialogFooter className="flex-wrap gap-1">
          <Button variant="outline" onClick={close}>
            Cancel
          </Button>
          {canRetry && (
            <Button variant="secondary" onClick={retry}>
              Preview again
            </Button>
          )}
          {ready &&
            stopFirst.map((service) => (
              <Button
                key={service}
                variant="secondary"
                data-stop-first={service}
                onClick={() => {
                  stopping.current = true
                  requestOp({ kind: 'op', stack, op: { kind: 'service_stop', options: { service } } })
                }}
              >
                Stop {service} first
              </Button>
            ))}
          {ready &&
            restart.map((service) => (
              <Button key={service} variant="secondary" data-confirm-restart={service} disabled={busy} onClick={() => confirm({ then: 'restart', service })}>
                Save and restart {service}
              </Button>
            ))}
          {phase !== 'refused' && (
            <Button variant={destructive ? 'destructive' : 'default'} disabled={!ready} onClick={() => confirm()} data-confirm>
              {phase === 'confirming' ? 'Working…' : CONFIRM[op.kind]}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
