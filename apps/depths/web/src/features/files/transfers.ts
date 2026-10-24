/**
 * Downloads and uploads (spec 053, US3, T051).
 *
 * - A file downloads from its link (`downloadHref`). A folder downloads as a tar from the same
 *   route, after a `FolderSize` pre-check: over `limits.files.download_max` (or a count the helper
 *   capped) it says how big it is and starts nothing.
 * - An upload previews first: `PreviewFileOp{upload}` names the folder and every file, marking the
 *   ones it replaces, and nothing is sent until that is confirmed. Then one
 *   `POST /api/v1/stacks/{stack}/files/upload?preview=&name=` per file, in order, carrying the
 *   session's CSRF token, with its progress here.
 */
import { create } from 'zustand'

import { useSession } from '@/app/session-store'
import type { FileOp } from '@/protocol/generated'

import { useFileOps } from './fileops-store'
import { downloadHref, filesLimits } from './model'
import { askFolderSize, refusalText, relist, type FilesRefusal } from './store'

export type UploadState = 'queued' | 'sending' | 'done' | 'failed'

export interface UploadItem {
  name: string
  size: number
  sent: number
  state: UploadState
  /** Why it failed, in words. */
  error?: string
}

export interface UploadJob {
  stack: string
  mount: string
  dir: string
  items: UploadItem[]
  /** Every file is done or failed. */
  finished: boolean
}

/** A folder download that did not start, and why; or one that did. */
export type DownloadNote =
  | { kind: 'checking'; path: string }
  | { kind: 'started'; path: string; bytes: number }
  | { kind: 'too_large'; path: string; bytes: number; capped: boolean; max: number }
  | { kind: 'refused'; path: string; text: string }

export interface TransfersState {
  upload: UploadJob | null
  download: DownloadNote | null
  clearUpload: () => void
  clearDownload: () => void
}

export const useTransfers = create<TransfersState>()((set) => ({
  upload: null,
  download: null,
  clearUpload: () => set({ upload: null }),
  clearDownload: () => set({ download: null }),
}))

/** Starts a browser download of `href` as `name`, as a click on a download link would. */
function startDownload(href: string, name: string): void {
  const a = document.createElement('a')
  a.href = href
  a.download = name
  a.rel = 'noopener'
  document.body.append(a)
  a.click()
  a.remove()
}

/** Downloads the folder `path` as a tar, if it is under the server's limit. */
export function downloadFolder(stack: string, mount: string, path: string, name: string): void {
  useTransfers.setState({ download: { kind: 'checking', path } })
  void askFolderSize(mount, path).then((r) => {
    if (useTransfers.getState().download?.path !== path) return
    if (!r.ok) {
      const text = r.refusal.kind === 'lost' ? 'The connection dropped before the size came back. Try again.' : refusalText(r.refusal)
      useTransfers.setState({ download: { kind: 'refused', path, text } })
      return
    }
    const max = filesLimits(useSession.getState().me)?.download_max ?? Infinity
    if (r.value.capped || r.value.bytes > max) {
      useTransfers.setState({ download: { kind: 'too_large', path, bytes: r.value.bytes, capped: r.value.capped, max } })
      return
    }
    useTransfers.setState({ download: { kind: 'started', path, bytes: r.value.bytes } })
    startDownload(downloadHref(stack, mount, path), `${name}.tar`)
  })
}

/** What an upload route's refusal body says, in words. */
function uploadError(status: number, body: string): string {
  try {
    const j = JSON.parse(body) as { kind?: string; error?: string }
    if (j.kind === 'size_mismatch') return 'The file changed size while it was sent.'
    if (j.kind) return refusalText(j as FilesRefusal)
    if (j.error === 'csrf') return 'The session check failed. Reload the page and try again.'
  } catch {
    // Not JSON: fall through.
  }
  return `The server answered ${String(status)}.`
}

function patchItem(i: number, patch: Partial<UploadItem>): void {
  const job = useTransfers.getState().upload
  if (!job) return
  const items = job.items.map((it, k) => (k === i ? { ...it, ...patch } : it))
  useTransfers.setState({ upload: { ...job, items, finished: items.every((it) => it.state === 'done' || it.state === 'failed') } })
}

function post(stack: string, previewId: string, file: File, i: number): Promise<void> {
  return new Promise((done) => {
    const q = new URLSearchParams({ preview: previewId, name: file.name })
    const xhr = new XMLHttpRequest()
    xhr.open('POST', `/api/v1/stacks/${encodeURIComponent(stack)}/files/upload?${q.toString()}`)
    xhr.setRequestHeader('x-csrf-token', useSession.getState().me?.csrf_token ?? '')
    xhr.upload.onprogress = (e) => patchItem(i, { sent: e.loaded })
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) patchItem(i, { state: 'done', sent: file.size })
      else patchItem(i, { state: 'failed', error: uploadError(xhr.status, xhr.responseText) })
      done()
    }
    xhr.onerror = () => {
      patchItem(i, { state: 'failed', error: 'The connection dropped while sending.' })
      done()
    }
    patchItem(i, { state: 'sending' })
    xhr.send(file)
  })
}

/**
 * Previews uploading `files` into `dir`; on Confirm, sends them one at a time. Nothing is sent if
 * the preview is cancelled or refused.
 */
export function requestUpload(stack: string, mount: string, dir: string, files: readonly File[]): void {
  if (files.length === 0) return
  const op: FileOp = { kind: 'upload', options: { mount, dir, files: files.map((f) => ({ name: f.name, size: f.size })) } }
  useFileOps.getState().request(stack, op, (done) => {
    const items: UploadItem[] = files.map((f) => ({ name: f.name, size: f.size, sent: 0, state: 'queued' }))
    useTransfers.setState({ upload: { stack, mount, dir, items, finished: false } })
    void (async () => {
      for (const [i, f] of files.entries()) await post(stack, done.preview_id, f, i)
      relist()
    })()
  })
}
