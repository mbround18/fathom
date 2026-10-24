/**
 * The preview dialog (US2 T055, shared by US3): what an operation will run, before it runs.
 *
 * - Every step's argv exactly as the server returned it, one step per line, copyable, with its
 *   cwd and whether it reads the draft on stdin. Nothing here builds a command (FR-012).
 * - What it reaches: containers, networks, and volumes only when chosen.
 * - Destructive options as switches, off by default (FR-015). Switching one on asks the server
 *   for a new preview, so the argv shown is always the server's; a destructive preview also needs
 *   the stack's name typed before Confirm enables.
 * - Confirm sends the `preview_id`. Radix traps focus; Escape and Cancel dismiss without sending
 *   anything, so nothing runs.
 */
import { CopyIcon, WarningIcon } from '@phosphor-icons/react'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Switch,
} from '@companion/ui'
import { useId, useState, type ReactNode } from 'react'

import { useSession } from '@/app/session-store'
import type { Preview, StackOp, Step } from '@/protocol/generated'

import { argvLine, KIND_LABEL, refusalText } from './argv'
import { requestKind, requestStack, useOps, type Flow, type PreviewRequest } from './ops-store'

function copy(text: string): void {
  try {
    void globalThis.navigator.clipboard.writeText(text).catch(() => undefined)
  } catch {
    /* No clipboard (insecure context): the line is selectable text anyway. */
  }
}

function StepLine({ step, index }: { step: Step; index: number }) {
  const line = argvLine(step.argv)
  return (
    <li data-step={index} className="flex min-w-0 flex-col gap-0.5 border-t border-border py-1 first:border-t-0">
      <div className="flex min-w-0 items-start gap-1">
        <code
          data-argv={JSON.stringify(step.argv)}
          className="min-w-0 flex-1 font-mono text-xs break-all whitespace-pre-wrap text-foreground select-all"
        >
          {line}
        </code>
        <Button variant="ghost" size="icon-xs" aria-label={`Copy step ${String(index + 1)}`} onClick={() => copy(line)}>
          <CopyIcon />
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        in <span data-field="cwd" className="font-mono break-all text-foreground/90">{step.cwd}</span>
        {step.stdin === 'draft' && <span data-field="stdin">, reading the edited file on stdin</span>}
      </p>
    </li>
  )
}

function ReachList({ label, items, field }: { label: string; items: readonly string[]; field: string }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd data-reach={field} className="min-w-0 break-words text-foreground/90">
        {items.length === 0 ? 'none' : items.join(', ')}
      </dd>
    </>
  )
}

/** A unified diff, line by line, coloured by its first character (and labelled for screen readers). */
export function DiffText({ diff }: { diff: string }) {
  const lines = diff.replace(/\n$/, '').split('\n')
  return (
    <pre data-diff className="max-h-64 min-w-0 overflow-auto border border-border bg-background/40 p-1 font-mono text-xs">
      {lines.map((l, i) => {
        const header = l.startsWith('+++') || l.startsWith('---') || l.startsWith('@@')
        const tone = header
          ? 'text-muted-foreground'
          : l.startsWith('+')
            ? 'bg-status-ok/10 text-status-ok'
            : l.startsWith('-')
              ? 'bg-status-err/10 text-status-err'
              : 'text-foreground/80'
        const kind = header ? 'header' : l.startsWith('+') ? 'added' : l.startsWith('-') ? 'removed' : 'context'
        return (
          <div key={i} data-diff-line={kind} className={`whitespace-pre-wrap break-all ${tone}`}>
            {l === '' ? ' ' : l}
          </div>
        )
      })}
    </pre>
  )
}

function Option({
  label,
  hint,
  checked,
  onChange,
  disabled,
}: {
  label: string
  hint: string
  checked: boolean
  onChange: (v: boolean) => void
  disabled: boolean
}) {
  const id = useId()
  return (
    <div className="flex min-w-0 items-start gap-2">
      <Switch id={id} checked={checked} onCheckedChange={onChange} disabled={disabled} className="mt-0.5" />
      <label htmlFor={id} className="min-w-0 text-xs">
        <span className="font-medium">{label}</span>
        <span className="block text-muted-foreground">{hint}</span>
      </label>
    </div>
  )
}

/** The switches a kind offers. Removing volumes or images is admin only (FR-016, data-model). */
function Options({ request, admin, busy }: { request: PreviewRequest; admin: boolean; busy: boolean }) {
  const requestPreview = useOps((s) => s.requestPreview)
  if (request.kind !== 'op') return null
  const { op, stack } = request
  const reissue = (next: StackOp): void => requestPreview({ kind: 'op', stack, op: next })
  if (op.kind === 'down' && admin) {
    return (
      <fieldset className="flex flex-col gap-2 border border-border p-2">
        <legend className="px-1 text-xs text-muted-foreground">Destructive options</legend>
        <Option
          label="Remove named volumes"
          hint="Deletes the stack's volumes and the data in them. Off keeps them."
          checked={op.options.remove_volumes}
          disabled={busy}
          onChange={(v) => reissue({ kind: 'down', options: { ...op.options, remove_volumes: v } })}
        />
        <Option
          label="Remove images"
          hint="Deletes every image the stack's services use."
          checked={op.options.remove_images}
          disabled={busy}
          onChange={(v) => reissue({ kind: 'down', options: { ...op.options, remove_images: v } })}
        />
      </fieldset>
    )
  }
  if (op.kind === 'delete') {
    return (
      <fieldset className="flex flex-col gap-2 border border-border p-2">
        <legend className="px-1 text-xs text-muted-foreground">Destructive options</legend>
        <Option
          label="Also delete the stack directory"
          hint={`Removes ${stack}/ with its compose file and .env. Off keeps the files.`}
          checked={op.options.remove_directory}
          disabled={busy}
          onChange={(v) => reissue({ kind: 'delete', options: { remove_directory: v } })}
        />
      </fieldset>
    )
  }
  return null
}

/** What the switches turned on, named in words beside the argv (FR-015). */
function chosen(request: PreviewRequest): string[] {
  if (request.kind !== 'op') return []
  const { op } = request
  if (op.kind === 'down') {
    return [
      ...(op.options.remove_volumes ? ['named volumes will be removed'] : []),
      ...(op.options.remove_images ? ['images will be removed'] : []),
    ]
  }
  if (op.kind === 'delete' && op.options.remove_directory) return ['the stack directory will be deleted']
  return []
}

function kept(request: PreviewRequest): string | null {
  if (request.kind !== 'op') return null
  const { op } = request
  if (op.kind === 'down' && !op.options.remove_volumes) return 'Volumes are kept.'
  if (op.kind === 'delete' && !op.options.remove_directory) return 'The stack directory is kept.'
  return null
}

function PreviewBody({ preview, request }: { preview: Preview; request: PreviewRequest }) {
  const on = chosen(request)
  const keep = kept(request)
  return (
    <>
      <section aria-label="Commands" className="flex min-w-0 flex-col gap-1">
        <h3 className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          {preview.steps.length === 1 ? 'Command' : `Commands, in order (${String(preview.steps.length)})`}
        </h3>
        <ol data-steps className="min-w-0 border border-border bg-background/40 px-2">
          {preview.steps.map((s, i) => (
            <StepLine key={i} step={s} index={i} />
          ))}
        </ol>
      </section>
      {preview.file_diff !== null && (
        <section aria-label="File changes" className="flex min-w-0 flex-col gap-1">
          <h3 className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">Changes</h3>
          <DiffText diff={preview.file_diff} />
        </section>
      )}
      <section aria-label="Affects" className="flex min-w-0 flex-col gap-1">
        <h3 className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">Affects</h3>
        <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-2 text-xs">
          <ReachList label="Containers" items={preview.reaches.containers} field="containers" />
          <ReachList label="Networks" items={preview.reaches.networks} field="networks" />
          {preview.reaches.volumes.length > 0 && (
            <ReachList label="Volumes" items={preview.reaches.volumes} field="volumes" />
          )}
        </dl>
        {keep !== null && <p data-kept className="text-xs text-muted-foreground">{keep}</p>}
      </section>
      {(preview.destructive || on.length > 0) && (
        <p data-destructive role="note" className="flex items-start gap-1 border border-status-err/50 bg-status-err/10 p-1.5 text-xs text-status-err">
          <WarningIcon aria-hidden className="mt-0.5 size-3 shrink-0" />
          <span>Destructive{on.length > 0 ? `: ${on.join(', ')}` : ''}. This cannot be undone.</span>
        </p>
      )}
      <p className="text-xs text-muted-foreground">
        Valid once, until <span data-field="expires">{preview.expires_at}</span>.
      </p>
    </>
  )
}

function Body({ flow, admin }: { flow: Flow; admin: boolean }) {
  const { preview, request, phase, refusal } = flow
  let main: ReactNode
  if (preview) main = <PreviewBody preview={preview} request={request} />
  else if (phase === 'previewing') {
    main = (
      <p role="status" data-pane-state="loading" className="text-xs text-muted-foreground">
        Building the preview…
      </p>
    )
  } else main = null
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <Options request={request} admin={admin} busy={phase === 'previewing' || phase === 'confirming'} />
      {main}
      {refusal && (
        <p data-refusal={refusal.kind} role="alert" className="border border-status-warn/60 bg-status-warn/10 p-1.5 text-xs break-words text-status-warn">
          {refusalText(refusal)}
        </p>
      )}
    </div>
  )
}

function serviceOf(req: PreviewRequest): string | null {
  if (req.kind !== 'op') return null
  const { op } = req
  return op.kind === 'service_up' || op.kind === 'service_stop' || op.kind === 'service_restart' ? op.options.service : null
}

/** Refusals the editor shows in place, not in a dialog. */
function editorOwned(flow: Flow): boolean {
  return flow.request.kind === 'save' && (flow.refusal?.kind === 'validation' || flow.refusal?.kind === 'conflict')
}

export function PreviewDialog() {
  const flow = useOps((s) => s.flow)
  const confirm = useOps((s) => s.confirm)
  const dismiss = useOps((s) => s.dismiss)
  const retry = useOps((s) => s.retry)
  const admin = useSession((s) => s.me?.role === 'admin')
  const [typed, setTyped] = useState('')
  const [typedFor, setTypedFor] = useState<string | null>(null)
  const typedId = useId()

  const open = flow !== null && !editorOwned(flow)
  const previewId = flow?.preview?.preview_id ?? null
  // A new preview needs the name typed again.
  if (typedFor !== previewId) {
    setTypedFor(previewId)
    setTyped('')
  }
  if (!flow || !open) return null

  const stack = requestStack(flow.request)
  const kind = flow.preview?.kind ?? requestKind(flow.request)
  const service = serviceOf(flow.request)
  const destructive = flow.preview?.destructive ?? false
  const typedOk = !destructive || typed === stack
  const canConfirm = flow.phase === 'ready' && flow.preview !== null && typedOk
  const canRetry = flow.refusal?.kind === 'void' || flow.refusal?.kind === 'busy' || flow.refusal?.kind === 'lost'

  return (
    <Dialog open onOpenChange={(o) => !o && dismiss()}>
      <DialogContent
        data-preview={kind}
        className="max-h-[90dvh] overflow-y-auto sm:max-w-2xl"
        aria-describedby={undefined}
      >
        <DialogHeader>
          <DialogTitle className="font-display tracking-wider break-words">
            {KIND_LABEL[kind]} {service !== null ? `${service} in ` : ''}
            {stack}
          </DialogTitle>
          <DialogDescription>Nothing runs until you confirm.</DialogDescription>
        </DialogHeader>
        <Body flow={flow} admin={admin} />
        {destructive && flow.preview && (
          <div className="flex min-w-0 flex-col gap-1">
            <label htmlFor={typedId} className="text-xs">
              Type <span className="font-mono font-semibold">{stack}</span> to confirm
            </label>
            <Input
              id={typedId}
              value={typed}
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => setTyped(e.target.value)}
            />
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={dismiss}>
            Cancel
          </Button>
          {canRetry && (
            <Button variant="secondary" onClick={retry}>
              Preview again
            </Button>
          )}
          {flow.phase !== 'refused' && (
            <Button
              variant={destructive ? 'destructive' : 'default'}
              disabled={!canConfirm}
              onClick={confirm}
              data-confirm
            >
              {flow.phase === 'confirming' ? 'Starting…' : `Confirm ${KIND_LABEL[kind].toLowerCase()}`}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
