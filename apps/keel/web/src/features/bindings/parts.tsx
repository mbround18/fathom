/**
 * Pieces the bindings and receivers pages share (spec 055 T052): the workspace picker, the
 * confirm dialog for a `Preview`, and the error line.
 */
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@companion/ui'
import { useEffect, useId, type ReactNode } from 'react'

import { useSession } from '@/app/session-store'

import { FIELD, useRepos } from './store'

/** A labelled form field. */
export function Field({ label, children, id }: { label: string; children: ReactNode; id: string }) {
  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-[11px] tracking-wider text-muted-foreground uppercase">
        {label}
      </label>
      {children}
    </div>
  )
}

/** Picks one of the signed-in user's workspaces and loads its lists. */
export function WorkspacePicker() {
  const grants = useSession((s) => s.me?.grants ?? [])
  const workspace = useRepos((s) => s.workspace)
  const load = useRepos((s) => s.load)
  const id = useId()
  const first = grants[0]?.workspace ?? null
  const unique = [...new Map(grants.map((g) => [g.workspace, g.workspace_slug])).entries()]

  useEffect(() => {
    const current = workspace ?? first
    if (current) load(current)
  }, [workspace, first, load])

  if (unique.length === 0)
    return (
      <p data-pane-state="empty" className="text-xs text-muted-foreground">
        You have no workspace yet. An owner or admin grants one from Teams.
      </p>
    )
  return (
    <div className="flex items-center gap-2">
      <label htmlFor={id} className="text-[11px] tracking-wider text-muted-foreground uppercase">
        Workspace
      </label>
      <select
        id={id}
        className={FIELD}
        value={workspace ?? first ?? ''}
        onChange={(e) => {
          load(e.target.value)
        }}
      >
        {unique.map(([ws, slug]) => (
          <option key={ws} value={ws}>
            {slug}
          </option>
        ))}
      </select>
    </div>
  )
}

export function ErrorLine() {
  const error = useRepos((s) => s.error)
  const clear = useRepos((s) => s.clearError)
  if (!error) return null
  return (
    <p role="alert" className="flex items-center justify-between gap-2 border border-status-err/40 px-2 py-1 text-xs text-status-err">
      <span>{error}</span>
      <Button variant="ghost" size="xs" onClick={clear}>
        Dismiss
      </Button>
    </p>
  )
}

/** The confirm dialog for the pending preview. */
export function PreviewDialog() {
  const preview = useRepos((s) => s.preview)
  const confirm = useRepos((s) => s.confirm)
  const cancel = useRepos((s) => s.cancel)
  return (
    <Dialog
      open={preview !== null}
      onOpenChange={(open) => {
        if (!open) cancel()
      }}
    >
      {preview && (
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Confirm</DialogTitle>
            <DialogDescription data-preview-summary>{preview.summary}</DialogDescription>
          </DialogHeader>
          {preview.affected.length > 0 && (
            <ul aria-label="Affected" className="flex flex-col gap-1 text-xs">
              {preview.affected.map((a) => (
                <li key={a} className="font-mono break-all">
                  {a}
                </li>
              ))}
            </ul>
          )}
          {preview.step_up && (
            <p className="text-xs text-muted-foreground">This change asks for your passkey.</p>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={cancel}>
              Cancel
            </Button>
            <Button onClick={() => void confirm()}>Confirm</Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  )
}
