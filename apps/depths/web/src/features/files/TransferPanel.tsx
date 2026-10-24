/**
 * Where downloads and uploads report (spec 053, US3, T051): a folder download's size check, and
 * an upload's progress per file. It sits under the listing's toolbar; `aria-live` carries the
 * outcome to a screen reader.
 */
import { Button } from '@companion/ui'

import { formatBytes } from '@/features/stacks/metrics'

import { useTransfers, type DownloadNote, type UploadJob } from './transfers'

function DownloadLine({ note }: { note: DownloadNote }) {
  const clear = useTransfers((s) => s.clearDownload)
  const text =
    note.kind === 'checking'
      ? 'Checking the folder size…'
      : note.kind === 'started'
        ? `Downloading ${formatBytes(note.bytes)} as a tar archive.`
        : note.kind === 'too_large'
          ? `This folder is ${note.capped ? 'more than ' : ''}${formatBytes(note.bytes)}, over this server's download limit of ${formatBytes(note.max)}, so nothing was downloaded.`
          : note.text
  const bad = note.kind === 'too_large' || note.kind === 'refused'
  return (
    <div data-download-note={note.kind} className="flex min-w-0 flex-wrap items-center gap-2 text-xs">
      <p className={`min-w-0 flex-1 break-words ${bad ? 'text-status-warn' : 'text-muted-foreground'}`}>{text}</p>
      {note.kind !== 'checking' && (
        <Button size="sm" variant="ghost" onClick={clear}>
          Dismiss
        </Button>
      )}
    </div>
  )
}

function UploadList({ job }: { job: UploadJob }) {
  const clear = useTransfers((s) => s.clearUpload)
  const failed = job.items.filter((i) => i.state === 'failed').length
  const done = job.items.filter((i) => i.state === 'done').length
  return (
    <section aria-label="Upload" data-upload-progress={job.finished ? 'finished' : 'sending'} className="flex min-w-0 flex-col gap-1 border border-border p-1.5 text-xs">
      <ul className="flex min-w-0 flex-col gap-1">
        {job.items.map((it) => (
          <li key={it.name} data-upload-item={it.name} data-state={it.state} className="flex min-w-0 flex-col gap-0.5">
            <span className="flex min-w-0 flex-wrap items-baseline gap-x-2">
              <span className="min-w-0 font-mono break-all">{it.name}</span>
              <span className="text-muted-foreground tabular-nums">
                {formatBytes(it.sent)} of {formatBytes(it.size)}
              </span>
              {it.state === 'failed' && <span className="text-status-err">{it.error}</span>}
            </span>
            <progress aria-label={`Uploading ${it.name}`} max={Math.max(it.size, 1)} value={it.state === 'done' ? Math.max(it.size, 1) : it.sent} className="h-1 w-full" />
          </li>
        ))}
      </ul>
      {job.finished && (
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span className={failed > 0 ? 'text-status-warn' : 'text-status-ok'}>
            {failed > 0 ? `Uploaded ${String(done)} of ${String(job.items.length)} files.` : `Uploaded ${String(done)} ${done === 1 ? 'file' : 'files'}.`}
          </span>
          <Button size="sm" variant="ghost" onClick={clear}>
            Dismiss
          </Button>
        </div>
      )}
    </section>
  )
}

export function TransferPanel({ stack, mount }: { stack: string; mount: string }) {
  const download = useTransfers((s) => s.download)
  const upload = useTransfers((s) => s.upload)
  const ours = upload && upload.stack === stack && upload.mount === mount ? upload : null
  return (
    <div role="status" aria-live="polite" data-transfers className="flex min-w-0 flex-col gap-1 empty:hidden">
      {download && <DownloadLine note={download} />}
      {ours && <UploadList job={ours} />}
    </div>
  )
}
