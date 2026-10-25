/**
 * The open file (spec 053, T037, T045): a header with the path, owner and permissions, then the
 * text in the shared CodeEditor (language guessed from the name and content, line numbers, soft
 * wrap), or, for a binary, too-large or non-UTF-8 file, its size and type and a download.
 *
 * The editor is read-only, with the reason shown, when the mount is read-only in every container,
 * the server's files feature is read-only, or the server marks the file write-blocked (a hard
 * link, a special file, a link out of the mount).
 *
 * Saving (US2): **Save** previews the change (`PreviewFileOp{save}`, the server's diff) and the
 * shared `FileOpDialog` confirms it. The save carries the fingerprint the file was opened at, so a
 * file that changed on disk meanwhile is refused (`changed_on_disk`); the editor then offers to
 * reload (the edit is lost) or to compare the edit with the file as it is now and save over it.
 * Parse problems (JSON, YAML, TOML, XML, INI) are marked on their lines as warnings; saving with
 * any asks once more. Leaving with unsaved changes asks to confirm.
 *
 * History (US5, T063): **History** in the header lists the file's kept versions (`HistoryPanel`).
 */
import { ArrowLeftIcon, ClockCounterClockwiseIcon, DownloadSimpleIcon, FloppyDiskIcon, WarningIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { useMemo, useState } from 'react'

import { Link } from '@/app/Link'
import { CodeEditor } from '@/features/editor/CodeEditor'
import { useUnsavedGuard } from '@/features/editor/useUnsavedGuard'
import { formatBytes } from '@/features/stacks/metrics'
import { useOps } from '@/features/stacks/ops-store'
import { DiffText } from '@/features/stacks/PreviewDialog'
import type { FileContent, Mount, WriteBlocked } from '@/protocol/generated'

import { lineDiff } from './diff'
import { isSaveConflict, useFileOps } from './fileops-store'
import { HistoryPanel } from './HistoryPanel'
import { guessLanguage } from './lang'
import { parseProblems } from './lint'
import { downloadHref, filesHref, modeString, parentPath, pathParts, type FilesMode } from './model'
import { fetchFile, fileSaved, refusalText, relist, reopen, type OpenFile } from './store'

const BLOCKED: Record<WriteBlocked, string> = {
  hard_link: 'It has more than one hard link, so an edit would change the other names too.',
  read_only_mount: 'This mount is read-only in every container that uses it.',
  special: 'It is a special file.',
  outside_link: 'It is a link that points outside the mount.',
}

const BINARY: Record<NonNullable<FileContent['binary_reason']>, string> = {
  binary: 'Binary file',
  too_large: 'Too large to open here',
  not_utf8: 'Not UTF-8 text',
}

/** Why the editor refuses edits, or `null` when it does not. */
function readOnlyReason(content: FileContent, mount: Mount | undefined, mode: FilesMode): string | null {
  if (mode === 'read_only') return 'Files are read-only on this server.'
  if (mount && !mount.writable) return BLOCKED.read_only_mount
  if (content.read_only) return BLOCKED[content.read_only]
  return null
}

export interface FileEditorProps {
  stack: string
  mount: string
  mountInfo: Mount | undefined
  /** The root's name in the way back: the volume name or host path. */
  rootLabel: string
  file: Exclude<OpenFile, { state: 'closed' }>
  mode: FilesMode
}

export function FileEditor({ stack, mount, mountInfo, rootLabel, file, mode }: FileEditorProps) {
  const parent = parentPath(file.path)
  const name = pathParts(file.path)?.at(-1) ?? file.path
  const folder = pathParts(parent)?.at(-1) ?? rootLabel
  const content = file.state === 'open' ? file.content : null
  const binary = content?.binary_reason ?? null
  // Kept outside the keyed text view, which starts afresh after every save.
  const [savedPath, setSavedPath] = useState<string | null>(null)
  // The file whose history is showing, so it closes when another file opens.
  const [historyOf, setHistoryOf] = useState<string | null>(null)

  return (
    <div data-pane="file" data-path={file.path} data-binary={binary ?? undefined} className="flex min-w-0 flex-col gap-1">
      <Link
        to={filesHref(stack, { mount, path: parent })}
        className="inline-flex h-7 min-w-0 items-center gap-1 self-start text-xs text-primary hover:underline"
      >
        <ArrowLeftIcon aria-hidden className="size-3.5 shrink-0" />
        <span className="min-w-0 truncate">Back to {folder}</span>
      </Link>

      <header data-file-header className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5 border-b border-border pb-1 text-xs">
        <h2 className="min-w-0 font-mono font-semibold break-all">{file.path}</h2>
        {content && (
          <span className="text-muted-foreground">
            {content.owner.uid}:{content.owner.gid} <span className="font-mono">{modeString(content.owner.mode)}</span>
          </span>
        )}
        <span className="flex-1" />
        {content && (
          <Button size="sm" variant="ghost" data-history-toggle aria-expanded={historyOf === file.path} onClick={() => setHistoryOf(historyOf === file.path ? null : file.path)}>
            <ClockCounterClockwiseIcon aria-hidden className="size-3.5" />
            History
          </Button>
        )}
      </header>
      {content && historyOf === file.path && (
        <HistoryPanel
          stack={stack}
          mount={mount}
          path={file.path}
          canRestore={mode === 'full' && mountInfo?.writable !== false}
          onRestored={() => {
            reopen()
            relist()
          }}
        />
      )}

      {file.state === 'loading' ? (
        <p role="status" data-pane-state="loading" className="text-xs text-muted-foreground">
          Opening…
        </p>
      ) : file.state === 'refused' ? (
        <p role="alert" data-refusal={file.refusal.kind} className="border border-border p-2 text-xs break-words text-status-err">
          {refusalText(file.refusal)}
        </p>
      ) : binary !== null || content?.text === undefined ? (
        <div className="flex min-w-0 flex-col items-start gap-2 border border-border p-3 text-xs">
          <p>
            <span className="font-medium">{BINARY[binary ?? 'binary']}</span>
            <span className="text-muted-foreground"> · {formatBytes(file.content.size)}</span>
          </p>
          <p className="text-muted-foreground">It does not open in the editor; download it to look inside.</p>
          <a
            href={downloadHref(stack, mount, file.path)}
            download={name}
            className="inline-flex h-7 items-center gap-1 border border-input px-2 hover:border-primary hover:text-primary"
          >
            <DownloadSimpleIcon aria-hidden className="size-3.5" />
            Download
          </a>
        </div>
      ) : (
        <TextView
          key={`${file.path}\u0000${content.fingerprint}`}
          stack={stack}
          mount={mount}
          name={name}
          path={file.path}
          content={content}
          text={content.text}
          reason={readOnlyReason(content, mountInfo, mode)}
          onSaved={() => setSavedPath(file.path)}
        />
      )}
      <p role="status" aria-live="polite" data-save-status className="min-h-4 text-xs text-status-ok">
        {savedPath === file.path ? 'Saved.' : ''}
      </p>
    </div>
  )
}

interface TextViewProps {
  stack: string
  mount: string
  name: string
  path: string
  content: FileContent
  /** `content.text`, known to be present. */
  text: string
  reason: string | null
  onSaved: () => void
}

/** The editable text. Keyed by path and fingerprint, so a save or a reload starts it afresh. */
function TextView({ stack, mount, name, path, content, text, reason, onSaved }: TextViewProps) {
  const [draft, setDraft] = useState(text)
  const [askProblems, setAskProblems] = useState(false)
  const [disk, setDisk] = useState<FileContent | null>(null)
  const [comparing, setComparing] = useState<'idle' | 'loading' | 'failed'>('idle')
  const language = guessLanguage(name, text)
  const problems = useMemo(() => parseProblems(language, draft, name), [language, draft, name])
  const dirty = draft !== text
  useUnsavedGuard(dirty && reason === null)

  const flow = useFileOps((s) => s.flow)
  const request = useFileOps((s) => s.request)
  const dismiss = useFileOps((s) => s.dismiss)
  const ours = flow?.op.kind === 'save' && flow.op.options.path === path && flow.op.options.mount === mount
  const conflict = ours && isSaveConflict(flow)
  const busy = ours && !conflict

  const save = (base: string) => {
    const body = draft
    setAskProblems(false)
    request(stack, { kind: 'save', options: { mount, path, text: body, base, bom: content.bom, eol: content.save_eol } }, (done, choice) => {
      onSaved()
      fileSaved(path, body, done.fingerprint)
      relist()
      if (choice.then === 'restart') {
        useOps.getState().requestPreview({ kind: 'op', stack, op: { kind: 'service_restart', options: { service: choice.service } } })
      }
    })
  }

  const compare = () => {
    setComparing('loading')
    void fetchFile(mount, path).then((r) => {
      if (r.ok && r.value.text !== undefined) {
        setDisk(r.value)
        setComparing('idle')
      } else setComparing('failed')
    })
  }

  return (
    <div className="flex min-w-0 flex-col gap-1">
      {reason !== null && (
        <p data-read-only-reason className="text-xs break-words text-muted-foreground">
          Read-only: {reason}
        </p>
      )}
      {reason === null && content.eol === 'mixed' && (
        <p data-mixed-eol role="note" className="text-xs text-muted-foreground">
          This file mixes line endings; saving writes them all as {content.save_eol === 'crlf' ? 'CRLF (Windows)' : 'LF (Unix)'}.
        </p>
      )}
      {reason === null && (
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <Button size="sm" data-save disabled={!dirty || busy} onClick={() => (problems.length > 0 ? setAskProblems(true) : save(content.fingerprint))}>
            <FloppyDiskIcon aria-hidden className="size-3.5" />
            Save
          </Button>
          {dirty && (
            <Button size="sm" variant="ghost" data-discard onClick={() => setDraft(text)}>
              Discard changes
            </Button>
          )}
          <span data-dirty={dirty || undefined} className="text-xs text-muted-foreground">
            {dirty ? 'Unsaved changes' : ''}
          </span>
        </div>
      )}
      {askProblems && (
        <div
          data-save-anyway
          role="alert"
          className="flex min-w-0 flex-wrap items-center gap-2 border border-status-warn/60 bg-status-warn/10 p-1.5 text-xs text-status-warn"
        >
          <WarningIcon aria-hidden className="size-3.5 shrink-0" />
          <span className="min-w-0">
            {problems.length === 1 ? 'The file check found a problem' : `The file check found ${String(problems.length)} problems`}; the service may
            not read it.
          </span>
          <Button size="sm" variant="secondary" onClick={() => save(content.fingerprint)}>
            Save anyway
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setAskProblems(false)}>
            Keep editing
          </Button>
        </div>
      )}
      {conflict && (
        <div data-conflict role="alert" className="flex min-w-0 flex-col gap-2 border border-status-warn/60 bg-status-warn/10 p-2 text-xs">
          <p className="text-status-warn">This file changed on disk since you opened it, so it was not saved.</p>
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="secondary"
              data-reload
              onClick={() => {
                dismiss()
                reopen()
              }}
            >
              Reload (lose my edit)
            </Button>
            <Button size="sm" variant="secondary" data-compare disabled={comparing === 'loading'} onClick={compare}>
              Compare
            </Button>
            <Button size="sm" variant="ghost" onClick={dismiss}>
              Keep editing
            </Button>
          </div>
          {comparing === 'failed' && <p className="text-status-err">The file could not be read again. Try again.</p>}
          {disk && (
            <section aria-label="On disk now, then your edit" data-compare-diff className="flex min-w-0 flex-col gap-1">
              <h3 className="font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">On disk now → your edit</h3>
              <DiffText diff={lineDiff(path, disk.text ?? '', draft) || 'No differences.'} />
              <Button size="sm" data-save-over className="self-start" onClick={() => save(disk.fingerprint)}>
                Save over it
              </Button>
            </section>
          )}
        </div>
      )}
      <CodeEditor value={draft} onChange={setDraft} language={language} label={path} problems={problems} readOnly={reason !== null} />
    </div>
  )
}
