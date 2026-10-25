/**
 * Asks for a name before a create, or for the new path before a rename or move (spec 053, US4,
 * T057). It sends nothing itself: submitting hands the op to `useFileOps`, whose `FileOpDialog`
 * shows the server's preview and confirms it.
 *
 * A rename takes a path relative to the mount, so a move to another folder of the same mount is
 * the same dialog; the mount is fixed and shown as the path's root, so no move across mounts is
 * offered (the server refuses one regardless).
 */
import { Button, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, Input } from '@companion/ui'
import { useRef, useState, type SyntheticEvent } from 'react'

import { noteOpener, returnFocus, type Opener } from './focus-return'
import { pathParts } from './model'

export type NameAsk = { kind: 'create'; type: 'file' | 'dir' } | { kind: 'rename'; from: string }

export interface NameDialogProps {
  ask: NameAsk
  /** The mount's root label: the volume name or host path. */
  rootLabel: string
  /** The folder a create lands in. */
  dir: string
  onCancel: () => void
  /** A create's name (one component), or a rename's new mount-relative path. */
  onSubmit: (value: string) => void
}

/** Why `value` cannot be submitted, or `null`. */
function problem(ask: NameAsk, value: string): string | null {
  const v = value.trim()
  if (v === '') return ask.kind === 'create' ? 'Give it a name.' : 'Give it a path.'
  if (ask.kind === 'create') {
    if (v.includes('/')) return 'A name cannot hold “/”.'
    if (v === '.' || v === '..') return 'That name is reserved.'
    return null
  }
  const parts = pathParts(v)
  if (parts === null || parts.length === 0) return 'The path must stay inside this mount (no “..”).'
  if (parts.join('/') === ask.from) return 'That is where it is now.'
  return null
}

export function NameDialog({ ask, rootLabel, dir, onCancel, onSubmit }: NameDialogProps) {
  const [value, setValue] = useState(ask.kind === 'rename' ? ask.from : '')
  const [tried, setTried] = useState(false)
  const opener = useRef<Opener | null>(null)
  const why = problem(ask, value)
  const title = ask.kind === 'rename' ? 'Rename or move' : ask.type === 'dir' ? 'New folder' : 'New file'
  const label = ask.kind === 'rename' ? 'New path' : 'Name'

  const submit = (ev: SyntheticEvent) => {
    ev.preventDefault()
    setTried(true)
    if (why !== null) return
    const v = value.trim()
    onSubmit(ask.kind === 'rename' ? (pathParts(v) ?? []).join('/') : v)
  }

  return (
    <Dialog open onOpenChange={(o) => !o && onCancel()}>
      <DialogContent
        data-name-dialog={ask.kind === 'rename' ? 'rename' : ask.type}
        className="sm:max-w-lg"
        onOpenAutoFocus={() => {
          opener.current = noteOpener()
        }}
        onCloseAutoFocus={(ev) => {
          ev.preventDefault()
          returnFocus(opener.current)
        }}
      >
        <DialogHeader>
          <DialogTitle className="font-display tracking-wider">{title}</DialogTitle>
          <DialogDescription>
            {ask.kind === 'rename'
              ? `The path inside ${rootLabel}. Change the folder part to move it; it stays in this mount.`
              : `In ${[rootLabel, ...(pathParts(dir) ?? [])].join('/')}. You confirm the change on the next step.`}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="flex min-w-0 flex-col gap-2" noValidate>
          <label className="flex min-w-0 flex-col gap-1 text-xs">
            <span>{label}</span>
            <span className="flex min-w-0 items-center gap-1">
              {ask.kind === 'rename' && (
                <span data-mount-root className="shrink-0 font-mono text-muted-foreground">
                  {rootLabel}/
                </span>
              )}
              <Input
                value={value}
                onChange={(e) => setValue(e.target.value)}
                aria-invalid={tried && why !== null}
                aria-describedby={tried && why !== null ? 'name-dialog-problem' : undefined}
                spellCheck={false}
                autoComplete="off"
                className="min-w-0 flex-1 font-mono text-xs"
              />
            </span>
          </label>
          {tried && why !== null && (
            <p id="name-dialog-problem" role="alert" className="text-xs text-status-err">
              {why}
            </p>
          )}
          <DialogFooter className="flex-wrap gap-1">
            <Button type="button" variant="outline" onClick={onCancel}>
              Cancel
            </Button>
            <Button type="submit" data-name-submit>
              Preview
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
