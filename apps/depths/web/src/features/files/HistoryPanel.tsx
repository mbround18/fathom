/**
 * A file's kept versions (spec 053, US5, T063): newest first, each with when, who and why it was
 * kept. **Compare** shows the file as it is now against that version in the shared diff view;
 * **Restore** previews putting it back (`PreviewFileOp{restore}`) and the shared `FileOpDialog`
 * confirms it.
 *
 * It works on a path that no longer exists too: a deleted file's content is kept as a version, so
 * the listing's **Deleted files** entry opens this panel for a name deleted in this session (kept
 * in `deleted.ts`), or for one typed in.
 */
import { ClockCounterClockwiseIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { useEffect, useState } from 'react'

import { formatBytes } from '@/features/stacks/metrics'
import { formatTime } from '@/features/stacks/model'
import { DiffText } from '@/features/stacks/PreviewDialog'
import type { FileVersion, VersionReason } from '@/protocol/generated'

import { lineDiff } from './diff'
import { useFileOps } from './fileops-store'
import { deletedIn, forgetDeleted, useDeleted } from './deleted'
import { joinPath } from './model'
import { askHistory, fetchFile, refusalText, type Fetched } from './store'

const REASON: Record<VersionReason, string> = {
  save: 'before a save',
  upload: 'before an upload replaced it',
  delete: 'when it was deleted',
  restore: 'before a restore',
}

export interface HistoryPanelProps {
  stack: string
  mount: string
  path: string
  /** Whether a version may be restored: the files feature is full and the mount writable. */
  canRestore: boolean
  /** After a restore lands. */
  onRestored: () => void
}

type Compare = { id: string; state: 'loading' } | { id: string; state: 'ready'; diff: string } | { id: string; state: 'failed'; text: string }

export function HistoryPanel({ stack, mount, path, canRestore, onRestored }: HistoryPanelProps) {
  const request = useFileOps((s) => s.request)
  const [versions, setVersions] = useState<Fetched<FileVersion[]> | null>(null)
  const [compare, setCompare] = useState<Compare | null>(null)
  const [round, setRound] = useState(0)

  useEffect(() => {
    let live = true
    void askHistory(mount, path).then((r) => {
      if (live) setVersions(r)
    })
    return () => {
      live = false
    }
  }, [mount, path, round])

  const show = (v: FileVersion) => {
    if (compare?.id === v.id && compare.state !== 'loading') {
      setCompare(null)
      return
    }
    setCompare({ id: v.id, state: 'loading' })
    void Promise.all([fetchFile(mount, path), fetchFile(mount, path, v.id)]).then(([now, then]) => {
      if (!then.ok || then.value.text === undefined) {
        const text = then.ok
          ? 'That version is not text, so it does not compare here.'
          : then.refusal.kind === 'lost'
            ? 'The connection dropped before that version came back. Try again.'
            : refusalText(then.refusal)
        setCompare({ id: v.id, state: 'failed', text })
        return
      }
      // A deleted file is not there now: compare against nothing.
      const current = now.ok ? (now.value.text ?? '') : ''
      setCompare({ id: v.id, state: 'ready', diff: lineDiff(path, current, then.value.text) })
    })
  }

  const restore = (v: FileVersion) => {
    request(stack, { kind: 'restore', options: { mount, path, version: v.id } }, () => {
      setCompare(null)
      setRound((n) => n + 1)
      onRestored()
    })
  }

  const list = versions?.ok ? versions.value : null
  return (
    <section aria-label={`History of ${path}`} data-history={path} className="flex min-w-0 flex-col gap-1 border border-border p-1.5 text-xs">
      <h3 className="flex items-center gap-1 font-display font-semibold tracking-wider text-muted-foreground uppercase">
        <ClockCounterClockwiseIcon aria-hidden className="size-3.5" />
        History
      </h3>
      {versions === null ? (
        <p role="status" data-pane-state="loading" className="text-muted-foreground">
          Loading the kept versions…
        </p>
      ) : !versions.ok ? (
        <p role="alert" data-refusal={versions.refusal.kind} className="break-words text-status-err">
          {versions.refusal.kind === 'lost' ? 'The connection dropped before the history came back. Try again.' : refusalText(versions.refusal)}
        </p>
      ) : list && list.length === 0 ? (
        <p data-pane-state="empty" className="text-muted-foreground">
          No earlier versions are kept for this file.
        </p>
      ) : (
        <ol aria-label="Kept versions, newest first" className="flex min-w-0 flex-col">
          {list?.map((v) => (
            <li key={v.id} data-version={v.id} data-reason={v.reason} className="flex min-w-0 flex-col gap-1 border-t border-border py-1 first:border-t-0">
              <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                <time dateTime={v.at} className="tabular-nums">
                  {formatTime(v.at)}
                </time>
                <span className="text-muted-foreground">
                  {REASON[v.reason]} · {formatBytes(v.size)} · by <span className="font-mono">{v.author}</span>
                </span>
                <span className="flex-1" />
                <Button size="sm" variant="ghost" data-version-compare aria-expanded={compare?.id === v.id} onClick={() => show(v)}>
                  Compare
                </Button>
                {canRestore && (
                  <Button size="sm" variant="outline" data-version-restore onClick={() => restore(v)}>
                    Restore…
                  </Button>
                )}
              </div>
              {compare?.id === v.id &&
                (compare.state === 'loading' ? (
                  <p role="status" className="text-muted-foreground">
                    Reading that version…
                  </p>
                ) : compare.state === 'failed' ? (
                  <p role="alert" className="text-status-err">
                    {compare.text}
                  </p>
                ) : (
                  <div data-version-diff className="flex min-w-0 flex-col gap-1">
                    <p className="text-muted-foreground">Now → this version</p>
                    {compare.diff === '' ? <p>Same as now.</p> : <DiffText diff={compare.diff} />}
                  </div>
                ))}
            </li>
          ))}
        </ol>
      )}
    </section>
  )
}

export interface DeletedFilesProps {
  stack: string
  mount: string
  dir: string
  canRestore: boolean
  /** After a deleted file is restored, with its path. */
  onRestored: (path: string) => void
}

/** The listing's way back to a deleted file: the ones deleted here this session, or a typed name. */
export function DeletedFiles({ stack, mount, dir, canRestore, onRestored }: DeletedFilesProps) {
  const names = useDeleted((s) => deletedIn(s.names, stack, mount, dir))
  const [picked, setPicked] = useState<string | null>(null)
  const [typed, setTyped] = useState('')
  const typedOk = typed.trim() !== '' && !typed.includes('/')

  return (
    <section aria-label="Deleted files" data-deleted className="flex min-w-0 flex-col gap-1 border border-border p-1.5 text-xs">
      {names.length > 0 ? (
        <ul aria-label="Deleted here in this session" className="flex min-w-0 flex-wrap gap-1">
          {names.map((n) => (
            <li key={n}>
              <Button size="sm" variant={picked === n ? 'secondary' : 'ghost'} data-deleted-file={n} aria-pressed={picked === n} onClick={() => setPicked(n)}>
                <span className="font-mono">{n}</span>
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-muted-foreground">No files were deleted here in this session.</p>
      )}
      <form
        className="flex min-w-0 flex-wrap items-center gap-1"
        onSubmit={(ev) => {
          ev.preventDefault()
          if (typedOk) setPicked(typed.trim())
        }}
      >
        <label htmlFor="deleted-name" className="text-muted-foreground">
          A file deleted earlier:
        </label>
        <input
          id="deleted-name"
          data-deleted-name
          value={typed}
          onChange={(ev) => setTyped(ev.target.value)}
          placeholder="its name"
          className="h-7 min-w-0 flex-1 border border-input bg-transparent px-2 font-mono"
        />
        <Button size="sm" variant="outline" type="submit" disabled={!typedOk}>
          Show history
        </Button>
      </form>
      {picked !== null && (
        <HistoryPanel
          key={picked}
          stack={stack}
          mount={mount}
          path={joinPath(dir, picked)}
          canRestore={canRestore}
          onRestored={() => {
            const path = joinPath(dir, picked)
            forgetDeleted(stack, mount, path)
            setPicked(null)
            onRestored(path)
          }}
        />
      )}
    </section>
  )
}
