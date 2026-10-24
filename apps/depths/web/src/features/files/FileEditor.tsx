/**
 * The open file (spec 053, T037), read path only: a header with the path, owner and permissions,
 * then the text in the shared CodeEditor (language guessed from the name and content, line
 * numbers, soft wrap), or, for a binary, too-large or non-UTF-8 file, its size and type and a
 * download.
 *
 * The editor is read-only, with the reason shown, when the mount is read-only in every container,
 * the server's files feature is read-only, or the server marks the file write-blocked (a hard
 * link, a special file, a link out of the mount). Saving lands in a later wave.
 */
import { ArrowLeftIcon, DownloadSimpleIcon } from '@phosphor-icons/react'

import { Link } from '@/app/Link'
import { CodeEditor } from '@/features/editor/CodeEditor'
import { formatBytes } from '@/features/stacks/metrics'
import type { FileContent, Mount, WriteBlocked } from '@/protocol/generated'

import { guessLanguage } from './lang'
import { downloadHref, filesHref, modeString, parentPath, pathParts, type FilesMode } from './model'
import { refusalText, type OpenFile } from './store'

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
      </header>

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
        <TextView name={name} path={file.path} text={content.text} reason={readOnlyReason(content, mountInfo, mode)} />
      )}
    </div>
  )
}

function TextView({ name, path, text, reason }: { name: string; path: string; text: string; reason: string | null }) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      {reason !== null && (
        <p data-read-only-reason className="text-xs break-words text-muted-foreground">
          Read-only: {reason}
        </p>
      )}
      <CodeEditor key={path} value={text} language={guessLanguage(name, text)} label={path} readOnly={reason !== null} />
    </div>
  )
}
