/**
 * The volume explorer's client state (spec 053, T034), over the shared socket.
 *
 * - `go(stack, location)` follows the URL (FR-024): it asks for the stack's mounts once, lists the
 *   folder the location names, or lists its parent and opens it when it is a file. A URL does not
 *   say whether its last part is a folder or a file, so the parent's listing decides; a link that
 *   stays inside the mount is tried as a folder first, and read as a file when that is refused.
 * - Every `ListDir` and `ReadFile` carries a fresh `req`. Pages and contents for a `req` that is no
 *   longer the current one are dropped (constitution V), so clicking quickly through folders
 *   never mixes two listings.
 * - Pages land at their `page` index, so the server's order (folders first, then by name) holds
 *   whatever order they arrive in.
 * - The open file follows data-model.md's state machine: `closed → loading → open`, or `refused`
 *   with the server's reason. Its draft, dirty state and save flow live in `FileEditor` and
 *   `fileops-store`; `fileSaved` records a save here, `reopen` reads it again (a reload).
 * - `fetchFile` and `fetchVersion` read a file (or a kept version) without touching the open one,
 *   for a conflict's compare and the history panel; `askHistory` lists a file's versions.
 * - `relist` re-reads the folder on screen after a change, keeping the rows until the new ones land.
 * - A reconnect asks again for what is on screen. Signing out forgets everything.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { DenyReason, Entry, FileContent, FileRefusal, FileVersion, Mount, RequestId } from '@/protocol/generated'

import { joinPath, parentPath, pathParts, type FilesLocation } from './model'

/** Why the server would not answer: a `Denied` gate, or a file-specific `FileRefused`. */
export type FilesRefusal = { kind: 'denied'; reason: DenyReason } | FileRefusal

export type ListStatus = 'idle' | 'loading' | 'ready' | 'refused'

export type OpenFile =
  | { state: 'closed' }
  | { state: 'loading'; path: string }
  | { state: 'open'; path: string; content: FileContent; dirty: false }
  | { state: 'refused'; path: string; refusal: FilesRefusal }

export interface FilesState {
  /** The stack whose mounts are held; `null` before the first `go`. */
  stack: string | null
  mounts: Mount[] | null
  mountsRefusal: FilesRefusal | null
  /** The mount being browsed; `null` on the mount list. */
  mount: string | null
  /** The folder listed (mount-relative, `''` for the root). */
  dir: string
  /** Every page received so far, in the server's order. */
  entries: Entry[]
  list: ListStatus
  listRefusal: FilesRefusal | null
  /** The name filter for the listed folder; a new folder starts unfiltered. */
  filter: string
  file: OpenFile
  go: (stack: string, location: FilesLocation) => void
  setFilter: (filter: string) => void
}

type Asked =
  | { what: 'mounts' }
  | { what: 'list'; req: number }
  | { what: 'probe'; req: number; path: string }
  | { what: 'file'; req: number }
  | { what: 'fetch'; req: number; done: (r: Fetched<FileContent>) => void }
  | { what: 'history'; done: (r: Fetched<FileVersion[]>) => void }
  | { what: 'size'; req: number; done: (r: Fetched<FolderSizeInfo>) => void }

/** A one-off read's answer: the value, or why not. */
export type Fetched<T> = { ok: true; value: T } | { ok: false; refusal: FilesRefusal | { kind: 'lost' } }

/** `FolderSize`'s answer: bytes and entries under a folder, `capped` when the count stopped early. */
export interface FolderSizeInfo {
  bytes: number
  entries: number
  capped: boolean
}

const CLOSED: OpenFile = { state: 'closed' }

const INITIAL = {
  stack: null,
  mounts: null,
  mountsRefusal: null,
  mount: null,
  dir: '',
  entries: [],
  list: 'idle' as ListStatus,
  listRefusal: null,
  filter: '',
  file: CLOSED,
}

/** Request id → what it asked, so `FileRefused` and `Denied` (which carry no `req`) find their owner. */
const pending = new Map<RequestId, Asked>()
let reqSeq = 0
/** The current `req` of each kind; a reply to any other is superseded. */
let listReq = -1
let probeReq = -1
let fileReq = -1
/** Pages of the current listing by index, and the index of the one marked `last` (-1 until then). */
let pages: Entry[][] = []
let lastPage = -1
/** The location `go` was last asked for, resolved once the parent's listing is in. */
let target: { mount: string; path: string } | null = null
let wanted: { stack: string; location: FilesLocation } | null = null

export const useFiles = create<FilesState>()((set, get) => ({
  ...INITIAL,

  go: (stack, location) => {
    wanted = { stack, location }
    if (get().stack !== stack) {
      clearRequests()
      set({ ...INITIAL, stack })
    }
    if (get().mounts === null && !asking('mounts')) askMounts(stack)
    if (location.mount === null) {
      target = null
      fileReq = -1
      set({ mount: null, file: CLOSED })
      return
    }
    resolve(location.mount, (pathParts(location.path) ?? []).join('/'))
  },

  setFilter: (filter) => set({ filter }),
}))

function asking(what: Asked['what']): boolean {
  for (const a of pending.values()) if (a.what === what) return true
  return false
}

function clearRequests(): void {
  pending.clear()
  listReq = -1
  probeReq = -1
  fileReq = -1
  pages = []
  lastPage = -1
  target = null
}

function send(asked: Asked, cmd: Parameters<typeof sendCmd>[0]): boolean {
  const id = sendCmd(cmd)
  if (id === null) return false
  pending.set(id, asked)
  return true
}

function askMounts(stack: string): void {
  send({ what: 'mounts' }, { type: 'ListMounts', data: { stack } })
}

/** Shows `path` in `mount`: its listing when it is a folder, its parent's and the file when not. */
function resolve(mount: string, path: string): void {
  const s = useFiles.getState()
  const sameMount = s.mount === mount
  if (path === '') {
    target = null
    closeFile()
    if (!(sameMount && s.dir === '' && s.list !== 'refused' && s.list !== 'idle')) list(mount, '')
    return
  }
  if (sameMount && s.dir === path && (s.list === 'loading' || s.list === 'ready')) {
    target = null
    closeFile()
    return
  }
  const file = s.file
  if (sameMount && file.state !== 'closed' && file.path === path) return
  target = { mount, path }
  if (sameMount && s.dir === parentPath(path) && (s.list === 'loading' || s.list === 'ready')) {
    if (s.list === 'ready') settleTarget()
    return
  }
  list(mount, parentPath(path))
}

/** The parent of `target` is listed: decide what the target is. */
function settleTarget(): void {
  const t = target
  const s = useFiles.getState()
  if (!t || t.mount !== s.mount || parentPath(t.path) !== s.dir) return
  target = null
  const name = pathParts(t.path)?.at(-1) ?? ''
  const e = s.entries.find((x) => x.name === name)
  if (e?.kind === 'dir') {
    closeFile()
    list(t.mount, t.path)
  } else if (e?.kind === 'file' || e?.kind === 'special' || (e?.kind === 'link' && !e.link?.inside)) {
    // Special files and outside links are refused by the server; the file pane says why.
    readFile(t.mount, t.path)
  } else {
    // An inside link, or a name not (yet) listed: a folder if the server lists it, else a file.
    probe(t.mount, t.path)
  }
}

function list(mount: string, path: string): void {
  reqSeq += 1
  listReq = reqSeq
  probeReq = -1
  pages = []
  lastPage = -1
  useFiles.setState({ mount, dir: path, entries: [], list: 'loading', listRefusal: null, filter: '' })
  const stack = useFiles.getState().stack ?? ''
  if (!send({ what: 'list', req: listReq }, { type: 'ListDir', data: { stack, mount, path, req: listReq } })) {
    useFiles.setState({ list: 'idle' })
  }
}

/** Asks for `path` as a folder without dropping the listing on screen. */
function probe(mount: string, path: string): void {
  reqSeq += 1
  probeReq = reqSeq
  const stack = useFiles.getState().stack ?? ''
  send({ what: 'probe', req: probeReq, path }, { type: 'ListDir', data: { stack, mount, path, req: probeReq } })
}

function readFile(mount: string, path: string): void {
  reqSeq += 1
  fileReq = reqSeq
  useFiles.setState({ file: { state: 'loading', path } })
  const stack = useFiles.getState().stack ?? ''
  send({ what: 'file', req: fileReq }, { type: 'ReadFile', data: { stack, mount, path, req: fileReq } })
}

function closeFile(): void {
  fileReq = -1
  if (useFiles.getState().file.state !== 'closed') useFiles.setState({ file: CLOSED })
}

function placePage(page: number, entries: Entry[]): Entry[] {
  pages[page] = entries
  // Holes (a later page before an earlier one) are skipped until the earlier one lands.
  return pages.flatMap((p) => p)
}

onServerEvent(({ reply_to, event }) => {
  if (reply_to === undefined) return
  const asked = pending.get(reply_to)
  if (!asked) return

  if (asked.what === 'fetch' || asked.what === 'history' || asked.what === 'size') {
    pending.delete(reply_to)
    const no: FilesRefusal | null =
      event.type === 'Denied' ? { kind: 'denied', reason: event.data.reason } : event.type === 'FileRefused' ? event.data : null
    if (asked.what === 'size') {
      if (event.type === 'FolderSize' && event.data.req === asked.req) {
        asked.done({ ok: true, value: { bytes: event.data.bytes, entries: event.data.entries, capped: event.data.capped } })
      } else if (no) asked.done({ ok: false, refusal: no })
      else pending.set(reply_to, asked)
    } else if (asked.what === 'fetch') {
      if (event.type === 'FileContent' && event.data.req === asked.req) asked.done({ ok: true, value: event.data.file })
      else if (no) asked.done({ ok: false, refusal: no })
      else pending.set(reply_to, asked)
    } else if (event.type === 'History') asked.done({ ok: true, value: event.data.versions })
    else if (no) asked.done({ ok: false, refusal: no })
    else pending.set(reply_to, asked)
    return
  }

  if (asked.what === 'mounts') {
    pending.delete(reply_to)
    if (event.type === 'Mounts') useFiles.setState({ mounts: event.data.mounts, mountsRefusal: null })
    else if (event.type === 'Denied') useFiles.setState({ mounts: [], mountsRefusal: { kind: 'denied', reason: event.data.reason } })
    else if (event.type === 'FileRefused') useFiles.setState({ mounts: [], mountsRefusal: event.data })
    return
  }

  const refusal: FilesRefusal | null =
    event.type === 'Denied' ? { kind: 'denied', reason: event.data.reason } : event.type === 'FileRefused' ? event.data : null

  if (asked.what === 'probe') {
    if (asked.req !== probeReq) {
      pending.delete(reply_to)
      return
    }
    if (event.type === 'DirPage') {
      // It is a folder: this reply becomes the listing.
      probeReq = -1
      listReq = asked.req
      pages = []
      lastPage = -1
      pending.set(reply_to, { what: 'list', req: asked.req })
      closeFile()
      useFiles.setState({ dir: asked.path, entries: [], list: 'loading', listRefusal: null, filter: '' })
      onPage(reply_to, event.data.page, event.data.entries, event.data.last)
    } else if (refusal) {
      pending.delete(reply_to)
      probeReq = -1
      const mount = useFiles.getState().mount
      if (mount !== null) readFile(mount, asked.path)
    }
    return
  }

  if (asked.what === 'list') {
    if (asked.req !== listReq) {
      pending.delete(reply_to)
      return
    }
    if (event.type === 'DirPage') {
      if (event.data.req !== listReq) return
      onPage(reply_to, event.data.page, event.data.entries, event.data.last)
    } else if (refusal) {
      pending.delete(reply_to)
      target = null
      useFiles.setState({ list: 'refused', listRefusal: refusal })
    }
    return
  }

  pending.delete(reply_to)
  const file = useFiles.getState().file
  if (asked.req !== fileReq || file.state === 'closed') return
  const { path } = file
  if (event.type === 'FileContent' && event.data.req === fileReq) {
    useFiles.setState({ file: { state: 'open', path, content: event.data.file, dirty: false } })
  } else if (refusal) {
    useFiles.setState({ file: { state: 'refused', path, refusal } })
  }
})

function onPage(replyTo: RequestId, page: number, entries: Entry[], last: boolean): void {
  const all = placePage(page, entries)
  if (last) lastPage = page
  // `reduce` skips holes, so this counts the pages that have landed.
  const complete = lastPage >= 0 && pages.reduce((n) => n + 1, 0) === lastPage + 1
  if (complete) pending.delete(replyTo)
  useFiles.setState({ entries: all, list: complete ? 'ready' : 'loading' })
  if (complete) settleTarget()
}

/** Reads `path` (or its kept `version`) once, leaving the open file as it is. */
export function fetchFile(mount: string, path: string, version?: string): Promise<Fetched<FileContent>> {
  return new Promise((done) => {
    reqSeq += 1
    const req = reqSeq
    const stack = useFiles.getState().stack ?? ''
    const sent =
      version === undefined
        ? send({ what: 'fetch', req, done }, { type: 'ReadFile', data: { stack, mount, path, req } })
        : send({ what: 'fetch', req, done }, { type: 'ReadVersion', data: { stack, mount, path, version, req } })
    if (!sent) done({ ok: false, refusal: { kind: 'lost' } })
  })
}

/** The kept versions of `path`, newest first. */
export function askHistory(mount: string, path: string): Promise<Fetched<FileVersion[]>> {
  return new Promise((done) => {
    const stack = useFiles.getState().stack ?? ''
    if (!send({ what: 'history', done }, { type: 'ListHistory', data: { stack, mount, path } })) done({ ok: false, refusal: { kind: 'lost' } })
  })
}

/** How much is under `path` (T051: the pre-check before a folder download). */
export function askFolderSize(mount: string, path: string): Promise<Fetched<FolderSizeInfo>> {
  return new Promise((done) => {
    reqSeq += 1
    const req = reqSeq
    const stack = useFiles.getState().stack ?? ''
    if (!send({ what: 'size', req, done }, { type: 'FolderSize', data: { stack, mount, path, req } })) done({ ok: false, refusal: { kind: 'lost' } })
  })
}

/** Reads the open file again from disk (a reload: the editor's draft goes with it). */
export function reopen(): void {
  const { mount, file } = useFiles.getState()
  if (mount === null || file.state === 'closed') return
  readFile(mount, file.path)
}

/** A save of the open file landed: its text and fingerprint are now these. */
export function fileSaved(path: string, text: string, fingerprint: string | undefined): void {
  const file = useFiles.getState().file
  if (file.state !== 'open' || file.path !== path) return
  const content: FileContent = { ...file.content, text, ...(fingerprint === undefined ? {} : { fingerprint }) }
  useFiles.setState({ file: { ...file, content } })
}

/** Lists the folder on screen again after a change, keeping its rows until the new ones land. */
export function relist(): void {
  const { mount, dir, list: status, stack } = useFiles.getState()
  if (mount === null || stack === null || status === 'idle') return
  reqSeq += 1
  listReq = reqSeq
  pages = []
  lastPage = -1
  if (!send({ what: 'list', req: listReq }, { type: 'ListDir', data: { stack, mount, path: dir, req: listReq } })) return
  useFiles.setState({ list: 'loading' })
}

/** Entries whose name holds `filter`, ignoring case, in listing order. */
export function filterEntries(entries: readonly Entry[], filter: string): readonly Entry[] {
  const q = filter.trim().toLowerCase()
  if (q === '') return entries
  return entries.filter((e) => e.name.toLowerCase().includes(q))
}

const count = (n: number): string => n.toLocaleString('en-US')

/** A refusal in words, for the listing, the file pane or a change's dialog. */
export function refusalText(r: FilesRefusal): string {
  switch (r.kind) {
    case 'denied':
      return r.reason === 'untrusted_network'
        ? 'Files are not available from this network.'
        : r.reason === 'feature_disabled'
          ? 'Files are turned off on this server.'
          : 'You are not allowed to browse these files.'
    case 'not_browsable':
      return `This mount cannot be browsed: ${r.reason.message}.`
    case 'helper_unavailable':
      return `The file helper could not start: ${r.reason}`
    case 'not_created':
      return 'This volume is not created yet: it appears after the first deploy.'
    case 'missing':
      return 'The source of this mount does not exist on the host.'
    case 'outside_mount':
      return 'This points outside the mount, so it does not open.'
    case 'not_found':
      return 'Not found: it may have been moved or deleted.'
    case 'not_text':
      return 'This is not a file that opens here.'
    case 'too_large':
      return r.bytes === undefined ? 'Too large to open here.' : `Too large: ${count(r.bytes)} bytes is over this server's limit.`
    case 'write_blocked':
      return 'This entry does not open here.'
    case 'feature_read_only':
      return 'Files are read-only on this server.'
    case 'read_only_mount':
      return 'This mount is read-only in every container that uses it.'
    case 'changed_on_disk':
      return 'The file changed on disk since it was opened.'
    case 'owner_changed':
      return 'The owner of this folder changed since it was listed. List it again.'
    case 'exists':
      return 'Something with that name already exists here.'
    case 'invalid_name':
      return 'That name is not allowed: no "/", no NUL, not "." or "..".'
    case 'mixed_owners':
      return 'The source and destination folders have different owners, so this cannot be moved there.'
    case 'not_confirmed':
      return 'This change was not confirmed. Preview it again.'
    case 'partial':
      return `Stopped after removing ${count(r.removed)} ${r.removed === 1 ? 'entry' : 'entries'}: ${r.failed_at} could not be removed: ${r.message} (${r.errno}).`
    case 'os_error':
      return `The system refused: ${r.message} (${r.errno}).`
    default:
      // A refusal newer than this client.
      return `Refused (${String((r as { kind: unknown }).kind).replaceAll('_', ' ')}).`
  }
}

/** The path of `entry` in the folder `dir`. */
export function entryPath(dir: string, entry: Entry): string {
  return joinPath(dir, entry.name)
}

/** Asks again for what is on screen (after a reconnect, or once the socket is up). */
function again(): void {
  const w = wanted
  if (!w) return
  clearRequests()
  useFiles.setState({ ...INITIAL, stack: w.stack })
  useFiles.getState().go(w.stack, w.location)
}

onReconnected(again)

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') {
    resetFilesStore()
    return
  }
  // Asked before the socket was up: ask again now that it is.
  if (s.connection === 'open' && prev.connection !== 'open' && wanted && useFiles.getState().mounts === null) again()
})

/** Test hook. */
export function resetFilesStore(): void {
  clearRequests()
  wanted = null
  useFiles.setState({ ...INITIAL })
}
