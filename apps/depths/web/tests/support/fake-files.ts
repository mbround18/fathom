/**
 * The fake backend's volume model (spec 053, T026): every stack's mounts, an in-memory tree of
 * folders and files behind each, kept versions, and the replies the server sends for the file
 * commands and the two file routes.
 *
 * It is pure (no Playwright, no Node APIs), so `FakeBackend` wires it into the mocked socket and
 * routes, and a unit test can drive it directly. Like the rest of the fake it reads no wall clock:
 * every write advances a counter that stands in for mtime, so fingerprints are stable per test.
 *
 * Usage, in a spec:
 *
 * ```ts
 * const data = backend.files.addMount('web', { source: 'web_data' })
 * backend.files.write('web', data, 'config/server.properties', 'motd=hi\n')
 * backend.files.seedFolder('web', data, 'logs', 12_000)
 * backend.files.setRunning('web', data, ['app'])
 * backend.files.refuseNext('changed_on_disk', 'Confirm')
 * ```
 */
import type {
  Cmd,
  CreateKind,
  Entry,
  Event,
  FileContent,
  FileLimits,
  FileOp,
  FileRefusal,
  FileVersion,
  Fingerprint,
  LinkInfo,
  Me,
  Mount,
  MountKind,
  Owner,
  PreviewVoidReason,
  UploadTarget,
  VersionReason,
  WriteBlocked,
} from '../../src/protocol/generated'

/** Entries per `DirPage`, as the server pages them (`depths_volumes::mount::PAGE_SIZE`). */
export const DIR_PAGE_SIZE = 5000
/** Where a folder delete stops counting ("10,000+"). */
export const DELETE_COUNT_CAP = 10_000
/** Kept versions per file. */
export const HISTORY_MAX = 20

const MIB = 1024 * 1024
/** What `fixtures.me()` reports in `limits.files`. */
export const FILE_LIMITS: FileLimits = { editor_max: 2 * MIB, download_max: 2048 * MIB, upload_max: 1024 * MIB }

/** 2026-10-23T12:00:00Z, the fake's `NOW`, in Unix seconds: the first write's mtime. */
const EPOCH_SECS = 1_792_756_800
const EXPIRES_AT = '2026-10-23T12:05:00Z'

export type FileCmd = Extract<Cmd, { type: 'ListMounts' | 'ListDir' | 'ReadFile' | 'FolderSize' | 'PreviewFileOp' | 'ListHistory' | 'ReadVersion' }>
export type FileCmdType = FileCmd['type']
/** What a scripted refusal can wait for: a file command, the `Confirm` of a file preview, or a route. */
export type RefusalTarget = FileCmdType | 'Confirm' | 'download' | 'upload'
type RefusalKind = FileRefusal['kind']

/** A route's answer, in the shape `route.fulfill` takes (the body still bytes or JSON). */
export interface FileHttpReply {
  status: number
  json?: unknown
  body?: Uint8Array
  contentType?: string
  headers?: Record<string, string>
}

interface Meta {
  uid: number
  gid: number
  mode: number
  nlink: number
  ino: number
  /** Whole seconds since the epoch, and the nanoseconds past it. */
  mtime: number
  nanos: number
}

type FakeNode =
  | { kind: 'file'; meta: Meta; data: Uint8Array; nameB64?: string }
  | { kind: 'dir'; meta: Meta; children: Map<string, FakeNode> }
  | { kind: 'link'; meta: Meta; link: LinkInfo }
  | { kind: 'special'; meta: Meta }
type DirNode = Extract<FakeNode, { kind: 'dir' }>
type FileNode = Extract<FakeNode, { kind: 'file' }>

interface FakeMount {
  stack: string
  mount: Mount
  root: DirNode
}

interface Kept {
  version: FileVersion
  data: Uint8Array
}

interface FilePreviewState {
  stack: string
  op: FileOp
  used: boolean
  expired: boolean
  /** `Upload`: the names still to be POSTed, with what the preview promised for each. */
  uploads: Map<string, UploadTarget>
}

/** Owner and link count a seeded entry gets unless the call says otherwise. */
export type EntryMeta = Partial<Owner & { nlink: number }>

const enc = new TextEncoder()
const bytes = (s: string): Uint8Array => enc.encode(s)
const BOM = [0xef, 0xbb, 0xbf]

// --- sha256 -----------------------------------------------------------------------------------

const K = [
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
  0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
  0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
  0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
  0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]

/**
 * sha256 of `data`, as lowercase hex. Written out so the model needs no Node or async Web Crypto;
 * fingerprints and mount ids then match what the server computes for the same bytes.
 */
export function sha256Hex(data: Uint8Array): string {
  const h = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]
  const len = data.length
  const buf = new Uint8Array(Math.ceil((len + 9) / 64) * 64)
  buf.set(data)
  buf[len] = 0x80
  const view = new DataView(buf.buffer)
  view.setUint32(buf.length - 8, Math.floor(len / 0x20000000))
  view.setUint32(buf.length - 4, (len * 8) >>> 0)
  const w = new Array<number>(64).fill(0)
  const at = (xs: number[], i: number): number => xs[i] ?? 0
  const ror = (x: number, n: number): number => (x >>> n) | (x << (32 - n))
  for (let off = 0; off < buf.length; off += 64) {
    for (let i = 0; i < 16; i++) w[i] = view.getUint32(off + i * 4)
    for (let i = 16; i < 64; i++) {
      const a = at(w, i - 15)
      const b = at(w, i - 2)
      const s0 = ror(a, 7) ^ ror(a, 18) ^ (a >>> 3)
      const s1 = ror(b, 17) ^ ror(b, 19) ^ (b >>> 10)
      w[i] = (at(w, i - 16) + s0 + at(w, i - 7) + s1) >>> 0
    }
    let [a, b, c, d, e, f, g, hh] = [at(h, 0), at(h, 1), at(h, 2), at(h, 3), at(h, 4), at(h, 5), at(h, 6), at(h, 7)]
    for (let i = 0; i < 64; i++) {
      const t1 = (hh + (ror(e, 6) ^ ror(e, 11) ^ ror(e, 25)) + ((e & f) ^ (~e & g)) + at(K, i) + at(w, i)) >>> 0
      const t2 = ((ror(a, 2) ^ ror(a, 13) ^ ror(a, 22)) + ((a & b) ^ (a & c) ^ (b & c))) >>> 0
      hh = g
      g = f
      f = e
      e = (d + t1) >>> 0
      d = c
      c = b
      b = a
      a = (t1 + t2) >>> 0
    }
    ;[a, b, c, d, e, f, g, hh].forEach((v, i) => {
      h[i] = (at(h, i) + v) >>> 0
    })
  }
  return h.map((v) => v.toString(16).padStart(8, '0')).join('')
}

// --- Fixtures and pure helpers ----------------------------------------------------------------

/** A mount's id as the server computes it: the first 16 hex digits of `sha256(kind \0 source)`. */
export function mountId(kind: MountKind, source: string): string {
  return sha256Hex(bytes(`${kind}\0${source}`)).slice(0, 16)
}

/** A ready, browsable, writable named volume mounted by `app` at `/data`, unless `over` says otherwise. */
export function fakeMount(source: string, over: Partial<Mount> = {}): Mount {
  const kind = over.kind ?? 'volume'
  return {
    id: mountId(kind, source),
    kind,
    source,
    services: [{ service: 'app', destination: '/data', read_only: false }],
    writable: true,
    state: 'ready',
    browsable: { ok: true },
    running: [],
    ...over,
  }
}

/** `sha256:<hex>:<size>:<mtime_ns>:<ino>` (R7). */
export function fingerprintOf(data: Uint8Array, meta: Pick<Meta, 'mtime' | 'nanos' | 'ino'>): Fingerprint {
  const ns = `${String(meta.mtime)}${String(meta.nanos).padStart(9, '0')}`
  return `sha256:${sha256Hex(data)}:${String(data.length)}:${ns}:${String(meta.ino)}`
}

/** RFC 3339 UTC to the second, as the helper writes `mtime`. */
function rfc3339(secs: number): string {
  return new Date(secs * 1000).toISOString().replace(/\.\d{3}Z$/, 'Z')
}

/** A path's components, as `RelPath` normalises them; `null` for `..` or a NUL (`outside_mount`). */
function partsOf(path: string): string[] | null {
  const parts = path.split('/').filter((p) => p !== '' && p !== '.')
  return parts.every((p) => p !== '..' && !p.includes('\0')) ? parts : null
}

const join = (parts: string[]): string => parts.join('/')

/** A single name `Create`, `Rename` and `Upload` accept (`Name`). */
function validName(name: string): boolean {
  return name !== '' && name !== '.' && name !== '..' && !name.includes('/') && !name.includes('\0') && bytes(name).length <= 255
}

/** Listing order: folders first, then names with ASCII case folded, then raw (`entry_order`). */
function entryOrder(a: Entry, b: Entry): number {
  const dir = (e: Entry): number => (e.kind === 'dir' ? 0 : 1)
  if (dir(a) !== dir(b)) return dir(a) - dir(b)
  const fold = (s: string): string => s.replace(/[A-Z]/g, (c) => c.toLowerCase())
  const x = fold(a.name)
  const y = fold(b.name)
  if (x !== y) return x < y ? -1 : 1
  return a.name < b.name ? -1 : a.name > b.name ? 1 : 0
}

/** Plain unified diff: every old line removed, every new line added. Enough for a fake. */
function naiveDiff(path: string, before: string, after: string): string {
  const a = before.split('\n').filter((l, i, all) => l !== '' || i < all.length - 1)
  const b = after.split('\n').filter((l, i, all) => l !== '' || i < all.length - 1)
  const body = [...a.filter((l) => !b.includes(l)).map((l) => `-${l}`), ...b.filter((l) => !a.includes(l)).map((l) => `+${l}`)]
  return [`--- a/${path}`, `+++ b/${path}`, `@@ -1,${String(a.length)} +1,${String(b.length)} @@`, ...body].join('\n') + '\n'
}

/** `data` as the editor gets it: text (LF, no BOM) when it is UTF-8 text within `editorMax`. */
function decode(data: Uint8Array, editorMax: number): Pick<FileContent, 'text' | 'binary_reason' | 'bom' | 'eol' | 'save_eol'> {
  const bom = BOM.every((b, i) => data[i] === b)
  const plain = { bom, eol: 'lf', save_eol: 'lf' } as const
  if (data.length > editorMax) return { ...plain, binary_reason: 'too_large' }
  const body = bom ? data.subarray(3) : data
  if (body.subarray(0, 8000).includes(0)) return { ...plain, binary_reason: 'binary' }
  let raw: string
  try {
    raw = new TextDecoder('utf-8', { fatal: true }).decode(body)
  } catch {
    return { ...plain, binary_reason: 'not_utf8' }
  }
  const crlf = (raw.match(/\r\n/g) ?? []).length
  const lf = (raw.match(/\n/g) ?? []).length - crlf
  const eol = crlf === 0 ? 'lf' : lf === 0 ? 'crlf' : 'mixed'
  return { text: raw.replace(/\r\n/g, '\n'), binary_reason: null, bom, eol, save_eol: eol === 'mixed' ? (crlf > lf ? 'crlf' : 'lf') : eol }
}

/** What a save writes: `text` with the ending and BOM it carries. */
function encodeSave(text: string, bom: boolean, eol: 'lf' | 'crlf'): Uint8Array {
  const body = bytes(eol === 'crlf' ? text.replace(/\n/g, '\r\n') : text)
  if (!bom) return body
  const out = new Uint8Array(body.length + 3)
  out.set(BOM)
  out.set(body, 3)
  return out
}

/** A plain ustar archive of `entries` (path, node), as the helper streams a folder download. */
function tar(entries: [string, FakeNode][]): Uint8Array {
  const blocks: Uint8Array[] = []
  const field = (h: Uint8Array, off: number, len: number, s: string): void => {
    h.set(bytes(s).subarray(0, len), off)
  }
  const octal = (n: number, len: number): string => n.toString(8).padStart(len - 1, '0') + '\0'
  for (const [path, node] of entries) {
    if (node.kind === 'special') continue
    const name = node.kind === 'dir' ? `${path}/` : path
    // ustar splits a long name at a `/` into a 155-byte prefix and a 100-byte name.
    const cut = name.length > 100 ? name.lastIndexOf('/', name.length - 2) : -1
    const [prefix, short] = cut > 0 ? [name.slice(0, cut), name.slice(cut + 1)] : ['', name]
    const data = node.kind === 'file' ? node.data : new Uint8Array(0)
    const h = new Uint8Array(512)
    field(h, 0, 100, short)
    field(h, 100, 8, octal(node.meta.mode, 8))
    field(h, 108, 8, octal(node.meta.uid, 8))
    field(h, 116, 8, octal(node.meta.gid, 8))
    field(h, 124, 12, octal(data.length, 12))
    field(h, 136, 12, octal(node.meta.mtime, 12))
    field(h, 148, 8, '        ')
    field(h, 156, 1, node.kind === 'dir' ? '5' : node.kind === 'link' ? '2' : '0')
    if (node.kind === 'link') field(h, 157, 100, node.link.target)
    field(h, 257, 8, 'ustar\u000000')
    field(h, 345, 155, prefix)
    field(h, 148, 8, octal(h.reduce((s, x) => s + x, 0), 7) + ' ')
    blocks.push(h, data, new Uint8Array((512 - (data.length % 512)) % 512))
  }
  blocks.push(new Uint8Array(1024))
  const out = new Uint8Array(blocks.reduce((s, b) => s + b.length, 0))
  let off = 0
  for (const b of blocks) {
    out.set(b, off)
    off += b.length
  }
  return out
}

/** A refusal kind with the fields it needs filled in, for `refuseNext('…')`. */
function refusalOf(kind: RefusalKind, current: Fingerprint | null): FileRefusal {
  switch (kind) {
    case 'helper_unavailable':
      return { kind, reason: 'the helper did not start' }
    case 'not_browsable':
      return { kind, reason: { code: 'risky_bind', message: 'it mounts a host system folder' } }
    case 'write_blocked':
      return { kind, reason: 'hard_link' }
    case 'changed_on_disk':
      return { kind, current: current ?? `sha256:${'0'.repeat(64)}:0:0:0` }
    case 'partial':
      return { kind, removed: 3, failed_at: 'locked.db', errno: 'EACCES', message: 'Permission denied' }
    case 'os_error':
      return { kind, errno: 'EIO', message: 'Input/output error' }
    default:
      return { kind }
  }
}

/** The status a route answers a refusal with (contracts/protocol.md § HTTP). */
function statusOf(r: FileRefusal): number {
  switch (r.kind) {
    case 'not_found':
    case 'missing':
      return 404
    case 'too_large':
      return 413
    case 'exists':
    case 'changed_on_disk':
      return 409
    case 'invalid_name':
      return 400
    default:
      return 403
  }
}

type Outcome<T> = { ok: true; value: T } | { ok: false; events: Event[] }
const refused = (r: FileRefusal): { ok: false; events: Event[] } => ({ ok: false, events: [{ type: 'FileRefused', data: r }] })

/**
 * The in-memory volumes. Seed them with `addMount`, `write`, `mkdir`, `symlink`, `special`,
 * `rawName` and `seedFolder`; read them back with `read`, `fingerprint` and `history`.
 */
export class FakeFiles {
  /** Entries per `DirPage`; lower it to page a small folder in a test. */
  pageSize = DIR_PAGE_SIZE

  private readonly byStack = new Map<string, FakeMount[]>()
  private readonly kept = new Map<string, Kept[]>()
  private readonly previews = new Map<string, FilePreviewState>()
  private readonly refusals: { refusal: FileRefusal | RefusalKind; on: RefusalTarget | null }[] = []
  private clock = 0
  private ino = 1000
  private versionSeq = 0
  private readonly me: () => Me | null
  private readonly nextPreviewId: () => string

  /**
   * @param me the session as it is now (the backend's `me`)
   * @param nextPreviewId a preview id from the backend's counter, so ids never clash with stack previews
   */
  constructor(me: () => Me | null, nextPreviewId: () => string) {
    this.me = me
    this.nextPreviewId = nextPreviewId
  }

  // --- Seeding --------------------------------------------------------------------------------

  /** Adds a mount to `stack` (`fakeMount(source, over)`) with an empty root, and returns its id. */
  addMount(stack: string, over: Partial<Mount> & { source?: string } = {}): string {
    const { source = `${stack}_data`, ...rest } = over
    const mount = fakeMount(source, rest)
    const list = this.byStack.get(stack) ?? []
    this.byStack.set(stack, [...list.filter((m) => m.mount.id !== mount.id), { stack, mount, root: this.dir({}) }])
    return mount.id
  }

  /** The mounts `ListMounts` answers for `stack`, as live objects: changing one changes the next reply. */
  mounts(stack: string): Mount[] {
    return (this.byStack.get(stack) ?? []).map((m) => m.mount)
  }

  /** Services with a running container that mount it: previews and `FileOpDone` name them (FR-010). */
  setRunning(stack: string, mount: string, services: string[]): void {
    this.mountOf(stack, mount).mount.running = services
  }

  /** Writes a file (parents made as needed), as another process on the host would: a new inode and mtime. */
  write(stack: string, mount: string, path: string, content: string | Uint8Array, meta: EntryMeta = {}): Fingerprint {
    const [parent, name] = this.parentFor(stack, mount, path)
    const prior = parent.children.get(name)
    const data = typeof content === 'string' ? bytes(content) : content
    const node: FileNode = { kind: 'file', meta: this.meta({ ...(prior ? prior.meta : {}), ...meta }), data }
    parent.children.set(name, node)
    return fingerprintOf(data, node.meta)
  }

  /** Makes a folder and its parents. */
  mkdir(stack: string, mount: string, path: string, meta: EntryMeta = {}): void {
    const [parent, name] = this.parentFor(stack, mount, path)
    if (parent.children.get(name)?.kind !== 'dir') parent.children.set(name, this.dir(meta))
  }

  /** A symbolic link; `inside: false` (it leaves the mount) is listed but never followed (R3). */
  symlink(stack: string, mount: string, path: string, target: string, inside = !target.startsWith('/') && partsOf(target) !== null): void {
    const [parent, name] = this.parentFor(stack, mount, path)
    parent.children.set(name, { kind: 'link', meta: this.meta({ mode: 0o777 }), link: { target, inside } })
  }

  /** A device, FIFO or socket: listed, never written. */
  special(stack: string, mount: string, path: string): void {
    const [parent, name] = this.parentFor(stack, mount, path)
    parent.children.set(name, { kind: 'special', meta: this.meta({ mode: 0o600 }) })
  }

  /** A file whose on-disk name is not UTF-8: listed as `display` with `name_b64`. */
  rawName(stack: string, mount: string, dir: string, display: string, nameB64: string, content: string | Uint8Array = ''): void {
    const [parent, name] = this.parentFor(stack, mount, join([...(partsOf(dir) ?? []), display]))
    const data = typeof content === 'string' ? bytes(content) : content
    parent.children.set(name, { kind: 'file', meta: this.meta({}), data, nameB64 })
  }

  /** `count` small files in `dir` (`file-00001.txt`, …), for paging and virtualised lists. */
  seedFolder(stack: string, mount: string, dir: string, count: number, name = (i: number) => `file-${String(i).padStart(5, '0')}.txt`): void {
    this.mkdir(stack, mount, dir)
    const folder = this.lookup(stack, mount, dir)
    if (folder?.kind !== 'dir') throw new Error(`fake-files: ${dir} is not a folder`)
    for (let i = 1; i <= count; i++) {
      folder.children.set(name(i), { kind: 'file', meta: this.meta({}), data: bytes(`line ${String(i)}\n`) })
    }
  }

  /** Removes an entry, as another process would. */
  remove(stack: string, mount: string, path: string): void {
    const [parent, name] = this.parentFor(stack, mount, path)
    parent.children.delete(name)
  }

  /** Keeps `content` as the newest version of `path`, as a save, upload, delete or restore would. */
  keepVersion(stack: string, mount: string, path: string, content: string, reason: VersionReason = 'save'): FileVersion {
    this.keep(stack, mount, path, bytes(content), reason)
    return this.history(stack, mount, path)[0] as FileVersion
  }

  // --- Reading back ---------------------------------------------------------------------------

  /** A file's content as text (`null` when it is not there). */
  read(stack: string, mount: string, path: string): string | null {
    const n = this.lookup(stack, mount, path)
    return n?.kind === 'file' ? new TextDecoder().decode(n.data) : null
  }

  /** A file's current fingerprint (`null` when it is not there). */
  fingerprint(stack: string, mount: string, path: string): Fingerprint | null {
    const n = this.lookup(stack, mount, path)
    return n?.kind === 'file' ? fingerprintOf(n.data, n.meta) : null
  }

  /** Whether anything is at `path`. */
  exists(stack: string, mount: string, path: string): boolean {
    return this.lookup(stack, mount, path) !== undefined
  }

  /** Kept versions of a file, newest first. */
  history(stack: string, mount: string, path: string): FileVersion[] {
    return (this.kept.get(this.key(stack, mount, path)) ?? []).map((k) => k.version)
  }

  // --- Scripting ------------------------------------------------------------------------------

  /**
   * The next file command (or the next one of `on`) is refused with `refusal`, after the role and
   * feature gates pass. A bare kind fills its fields: `changed_on_disk` names the target's current
   * fingerprint, `os_error` is `EIO`, and so on. Queued refusals are used in order.
   */
  refuseNext(refusal: FileRefusal | RefusalKind, on: RefusalTarget | null = null): this {
    this.refusals.push({ refusal, on })
    return this
  }

  /** Marks every outstanding file preview expired, as five minutes passing would. */
  expirePreviews(): void {
    for (const p of this.previews.values()) p.expired = true
  }

  /** Whether `previewId` is a file preview (its `Confirm` belongs here). */
  owns(previewId: string): boolean {
    return this.previews.has(previewId)
  }

  // --- Commands -------------------------------------------------------------------------------

  /** The reply to a file command, as the server's `files_gate` and helper would give it. */
  handle(cmd: FileCmd, may: (stack: string) => boolean): Event[] {
    const { stack } = cmd.data
    const write = cmd.type === 'PreviewFileOp'
    const gate = this.gate(stack, may, write, cmd.type, this.targetOf(cmd))
    if (gate) return gate
    if (cmd.type === 'ListMounts') return [{ type: 'Mounts', data: { stack, mounts: this.mounts(stack) } }]
    if (cmd.type === 'PreviewFileOp') return this.preview(stack, cmd.data.op)

    const { mount, path } = cmd.data
    const m = this.usable(stack, mount, false)
    if (!m.ok) return m.events
    const found = this.resolve(m.value, path)
    switch (cmd.type) {
      case 'ListDir': {
        if (!found.ok) return found.events
        const dir = found.value
        if (dir.kind !== 'dir') return refused({ kind: 'not_found' }).events
        const entries = [...dir.children].map(([name, n]) => this.entry(m.value, name, n)).sort(entryOrder)
        const pages = Math.max(1, Math.ceil(entries.length / this.pageSize))
        return Array.from({ length: pages }, (_, page): Event => ({
          type: 'DirPage',
          data: { req: cmd.data.req, mount, path, entries: entries.slice(page * this.pageSize, (page + 1) * this.pageSize), page, last: page === pages - 1 },
        }))
      }
      case 'ReadFile': {
        if (!found.ok) return found.events
        const n = found.value
        if (n.kind !== 'file') return refused({ kind: 'not_text' }).events
        return [{ type: 'FileContent', data: { req: cmd.data.req, mount, path, file: this.content(m.value, n) } }]
      }
      case 'FolderSize': {
        if (!found.ok) return found.events
        const max = this.limits().download_max
        let total = 0
        let entries = 0
        let capped = false
        const walk = (n: FakeNode): void => {
          if (capped) return
          entries++
          if (n.kind === 'file') total += n.data.length
          if (total > max) capped = true
          if (n.kind === 'dir') for (const c of n.children.values()) walk(c)
        }
        if (found.value.kind === 'dir') for (const c of found.value.children.values()) walk(c)
        else walk(found.value)
        return [{ type: 'FolderSize', data: { req: cmd.data.req, bytes: total, entries, capped } }]
      }
      case 'ListHistory':
        return [{ type: 'History', data: { mount, path, versions: this.history(stack, mount, path) } }]
      case 'ReadVersion': {
        const k = this.kept.get(this.key(stack, mount, path))?.find((v) => v.version.id === cmd.data.version)
        if (!k) return refused({ kind: 'not_found' }).events
        const file = found.ok && found.value.kind === 'file' ? found.value : null
        const meta = file?.meta ?? this.meta({})
        const content = this.content(m.value, { kind: 'file', meta, data: k.data })
        delete content.read_only
        return [{ type: 'FileContent', data: { req: cmd.data.req, mount, path, version: cmd.data.version, file: content } }]
      }
    }
  }

  /** `Confirm` of a file preview: applies the op, or says why not. */
  confirm(previewId: string, may: (stack: string) => boolean): Event[] {
    const p = this.previews.get(previewId)
    const voidWith = (reason: PreviewVoidReason): Event[] => [{ type: 'PreviewVoid', data: { reason } }]
    if (!p) return voidWith('unknown')
    if (p.used) return voidWith('used')
    if (p.expired) return voidWith('expired')
    const gate = this.gate(p.stack, may, true, 'Confirm', this.opTarget(p.stack, p.op))
    if (gate) return gate
    const m = this.usable(p.stack, p.op.options.mount, true)
    if (!m.ok) return m.events
    const applied = this.apply(p, m.value)
    if (!applied.ok) return applied.events
    p.used = true
    const { path, fingerprint } = applied.value
    return [
      {
        type: 'FileOpDone',
        data: { preview_id: previewId, mount: m.value.mount.id, path, ...(fingerprint ? { fingerprint } : {}), running: m.value.mount.running },
      },
    ]
  }

  // --- HTTP -----------------------------------------------------------------------------------

  /** `GET /api/v1/stacks/{stack}/files/download?mount=&path=`: a file's bytes, or a folder as a tar. */
  download(stack: string, query: URLSearchParams, may: (stack: string) => boolean): FileHttpReply {
    const mount = query.get('mount') ?? ''
    const path = query.get('path') ?? ''
    const gate = this.gate(stack, may, false, 'download', this.fingerprint(stack, mount, path))
    if (gate) return this.http(gate)
    const m = this.usable(stack, mount, false)
    if (!m.ok) return this.http(m.events)
    const found = this.resolve(m.value, path)
    if (!found.ok) return this.http(found.events)
    const n = found.value
    const parts = partsOf(path) ?? []
    const base = parts.at(-1) ?? m.value.mount.source.split('/').filter(Boolean).at(-1) ?? 'volume'
    const max = this.limits().download_max
    const headers = (filename: string): Record<string, string> => ({
      'cache-control': 'no-store',
      'x-content-type-options': 'nosniff',
      'content-disposition': `attachment; filename="${filename}"`,
    })
    if (n.kind === 'file') {
      if (n.data.length > max) return { status: 413, json: { kind: 'too_large', bytes: n.data.length } }
      return { status: 200, contentType: 'application/octet-stream', headers: headers(base), body: n.data }
    }
    if (n.kind !== 'dir') return { status: 404, json: { kind: 'not_found' } }
    const all: [string, FakeNode][] = []
    const walk = (prefix: string, d: DirNode): void => {
      all.push([prefix, d])
      for (const [name, c] of d.children) {
        if (c.kind === 'dir') walk(`${prefix}/${name}`, c)
        else all.push([`${prefix}/${name}`, c])
      }
    }
    walk(base, n)
    const size = all.reduce((s, [, c]) => s + (c.kind === 'file' ? c.data.length : 0), 0)
    if (size > max) return { status: 413, json: { kind: 'too_large', bytes: size } }
    return { status: 200, contentType: 'application/x-tar', headers: headers(`${base}.tar`), body: tar(all) }
  }

  /**
   * `POST /api/v1/stacks/{stack}/files/upload?preview=&name=`: one file of a confirmed `Upload`
   * preview. Mount, folder and the fingerprint it replaces come from the preview, never the query.
   */
  upload(stack: string, query: URLSearchParams, body: Uint8Array, may: (stack: string) => boolean): FileHttpReply {
    const p = this.previews.get(query.get('preview') ?? '')
    const name = query.get('name') ?? ''
    const op = p?.op.kind === 'upload' && p.stack === stack ? p.op.options : null
    const target = op ? join([...(partsOf(op.dir) ?? []), name]) : ''
    const gate = this.gate(stack, may, true, 'upload', op ? this.fingerprint(stack, op.mount, target) : null)
    if (gate) return this.http(gate)
    if (!p || !op || !p.used || p.expired) return { status: 403, json: { kind: 'not_confirmed' } }
    const promised = p.uploads.get(name)
    if (!promised) {
      return op.files.some((f) => f.name === name) ? { status: 403, json: { kind: 'not_confirmed' } } : { status: 400, json: { kind: 'invalid_name' } }
    }
    if (body.length > this.limits().upload_max) return { status: 413, json: { kind: 'too_large', bytes: body.length } }
    if (body.length !== promised.size) return { status: 400, json: { kind: 'size_mismatch' } }
    const m = this.usable(stack, op.mount, true)
    if (!m.ok) return this.http(m.events)
    const current = this.fingerprint(stack, op.mount, target)
    if (current !== null && !promised.exists) return { status: 409, json: { kind: 'exists' } }
    if (promised.exists && current !== null && current !== promised.fingerprint) {
      return { status: 409, json: { kind: 'changed_on_disk', current } }
    }
    p.uploads.delete(name)
    const prior = this.lookup(stack, op.mount, target)
    if (prior?.kind === 'file') this.keep(stack, op.mount, target, prior.data, 'upload')
    return { status: 201, json: { fingerprint: this.write(stack, op.mount, target, body) } }
  }

  // --- Internals ------------------------------------------------------------------------------

  private limits(): FileLimits {
    return this.me()?.limits?.files ?? FILE_LIMITS
  }

  /**
   * R12, in order: the feature mode (off is `Denied feature_disabled`), the role (edit rights on the
   * stack), read-only mode for writes, then any scripted refusal. Trust is the backend's gate.
   */
  private gate(stack: string, may: (stack: string) => boolean, write: boolean, what: RefusalTarget, current: Fingerprint | null): Event[] | null {
    const me = this.me()
    const mode = me?.features.files ?? 'off'
    if (mode === 'off') return [{ type: 'Denied', data: { reason: 'feature_disabled' } }]
    if (!me?.features.edit || !may(stack)) return [{ type: 'Denied', data: { reason: 'not_permitted' } }]
    if (write && mode === 'read_only') return refused({ kind: 'feature_read_only' }).events
    const i = this.refusals.findIndex((r) => r.on === null || r.on === what)
    if (i < 0) return null
    const [next] = this.refusals.splice(i, 1)
    if (!next) return null
    return refused(typeof next.refusal === 'string' ? refusalOf(next.refusal, current) : next.refusal).events
  }

  /** The file a command names, for a scripted `changed_on_disk`. */
  private targetOf(cmd: FileCmd): Fingerprint | null {
    if (cmd.type === 'ListMounts') return null
    if (cmd.type === 'PreviewFileOp') return this.opTarget(cmd.data.stack, cmd.data.op)
    return this.fingerprint(cmd.data.stack, cmd.data.mount, cmd.data.path)
  }

  private opTarget(stack: string, op: FileOp): Fingerprint | null {
    const o = op.options
    const path = 'path' in o ? o.path : 'from' in o ? o.from : null
    return path === null ? null : this.fingerprint(stack, o.mount, path)
  }

  /** A refusal event as a route's answer. */
  private http(events: Event[]): FileHttpReply {
    const [e] = events
    if (e?.type === 'Denied') return { status: 403, json: { kind: e.data.reason } }
    if (e?.type === 'FileRefused') return { status: statusOf(e.data), json: e.data }
    return { status: 500, json: { kind: 'os_error' } }
  }

  /** The mount, if it may be browsed now (and written, for a write). */
  private usable(stack: string, id: string, write: boolean): Outcome<FakeMount> {
    const m = this.byStack.get(stack)?.find((x) => x.mount.id === id)
    if (!m) return refused({ kind: 'not_found' })
    if (m.mount.state === 'not_created') return refused({ kind: 'not_created' })
    if (m.mount.state !== 'ready') return refused({ kind: 'missing' })
    if (!m.mount.browsable.ok) {
      return refused({ kind: 'not_browsable', reason: m.mount.browsable.reason ?? { code: 'other', message: 'not browsable' } })
    }
    if (write && !m.mount.writable) return refused({ kind: 'read_only_mount' })
    return { ok: true, value: m }
  }

  /** The node at `path`, following links that stay inside the mount. */
  private resolve(m: FakeMount, path: string, depth = 0): Outcome<FakeNode> {
    const parts = partsOf(path)
    if (parts === null || depth > 40) return refused({ kind: 'outside_mount' })
    let node: FakeNode = m.root
    for (const [i, part] of parts.entries()) {
      if (node.kind === 'link') {
        if (!node.link.inside) return refused({ kind: 'outside_mount' })
        const next = this.resolve(m, join([...parts.slice(0, i - 1), node.link.target]), depth + 1)
        if (!next.ok) return next
        node = next.value
      }
      if (node.kind !== 'dir') return refused({ kind: 'not_found' })
      const child = node.children.get(part)
      if (!child) return refused({ kind: 'not_found' })
      node = child
    }
    if (node.kind === 'link') {
      if (!node.link.inside) return refused({ kind: 'outside_mount' })
      return this.resolve(m, join([...parts.slice(0, -1), node.link.target]), depth + 1)
    }
    return { ok: true, value: node }
  }

  private entry(m: FakeMount, name: string, n: FakeNode): Entry {
    const link = n.kind === 'link' ? n.link : undefined
    const shape: WriteBlocked | null =
      n.kind === 'special' ? 'special' : link && !link.inside ? 'outside_link' : n.kind === 'file' && n.meta.nlink > 1 ? 'hard_link' : null
    const size = n.kind === 'file' ? n.data.length : n.kind === 'dir' ? 4096 : link ? bytes(link.target).length : 0
    return {
      name,
      ...(n.kind === 'file' && n.nameB64 ? { name_b64: n.nameB64 } : {}),
      kind: n.kind,
      ...(link ? { link } : {}),
      size,
      mtime: rfc3339(n.meta.mtime),
      uid: n.meta.uid,
      gid: n.meta.gid,
      mode: n.meta.mode,
      nlink: n.meta.nlink,
      write_blocked: shape ?? (m.mount.writable ? null : 'read_only_mount'),
    }
  }

  private content(m: FakeMount, n: FileNode): FileContent {
    const blocked: WriteBlocked | null = n.meta.nlink > 1 ? 'hard_link' : m.mount.writable ? null : 'read_only_mount'
    return {
      fingerprint: fingerprintOf(n.data, n.meta),
      size: n.data.length,
      ...decode(n.data, this.limits().editor_max),
      ...(blocked ? { read_only: blocked } : {}),
      owner: { uid: n.meta.uid, gid: n.meta.gid, mode: n.meta.mode },
    }
  }

  /** `PreviewFileOp`: checks what can be checked now, and holds the op for `Confirm`. */
  private preview(stack: string, op: FileOp): Event[] {
    const m = this.usable(stack, op.options.mount, true)
    if (!m.ok) return m.events
    const fm = m.value
    const at = (path: string): FakeNode | undefined => {
      const r = this.resolve(fm, path)
      return r.ok ? r.value : undefined
    }
    let summary: string
    let diff: string | null = null
    let entries: number | null = null
    let entriesCapped = false
    const uploads = new Map<string, UploadTarget>()
    switch (op.kind) {
      case 'save': {
        const n = at(op.options.path)
        if (n?.kind !== 'file') return refused({ kind: 'not_found' }).events
        if (n.meta.nlink > 1) return refused({ kind: 'write_blocked', reason: 'hard_link' }).events
        diff = naiveDiff(op.options.path, decode(n.data, Infinity).text ?? '', op.options.text)
        summary = `Save ${op.options.path}`
        break
      }
      case 'create': {
        const parent = at(op.options.parent)
        if (parent?.kind !== 'dir') return refused({ kind: 'not_found' }).events
        if (!validName(op.options.name)) return refused({ kind: 'invalid_name' }).events
        if (parent.children.has(op.options.name)) return refused({ kind: 'exists' }).events
        summary = `Create ${op.options.kind === 'dir' ? 'folder' : 'file'} ${join([...(partsOf(op.options.parent) ?? []), op.options.name])}`
        break
      }
      case 'rename': {
        const from = partsOf(op.options.from)
        const to = partsOf(op.options.to)
        if (!from || !to) return refused({ kind: 'outside_mount' }).events
        if (!this.lookupIn(fm, from)) return refused({ kind: 'not_found' }).events
        const name = to.at(-1) ?? ''
        if (!validName(name) || join(to).startsWith(`${join(from)}/`)) return refused({ kind: 'invalid_name' }).events
        if (this.lookupIn(fm, to.slice(0, -1))?.kind !== 'dir') return refused({ kind: 'not_found' }).events
        if (this.lookupIn(fm, to)) return refused({ kind: 'exists' }).events
        summary = `Rename ${join(from)} → ${join(to)}`
        break
      }
      case 'delete': {
        const n = this.deleteTarget(fm, op.options.path, op.options.raw_name)
        if (!n) return refused({ kind: 'not_found' }).events
        if (n.kind === 'dir') {
          let count = 0
          const walk = (d: DirNode): void => {
            for (const c of d.children.values()) {
              if (count >= DELETE_COUNT_CAP) return
              count++
              if (c.kind === 'dir') walk(c)
            }
          }
          walk(n)
          entries = count
          entriesCapped = count >= DELETE_COUNT_CAP
        }
        summary = `Delete ${op.options.path}`
        break
      }
      case 'restore': {
        const n = at(op.options.path)
        const k = this.kept.get(this.key(stack, fm.mount.id, op.options.path))?.find((v) => v.version.id === op.options.version)
        if (!k) return refused({ kind: 'not_found' }).events
        const current = n?.kind === 'file' ? (decode(n.data, Infinity).text ?? '') : ''
        diff = naiveDiff(op.options.path, current, decode(k.data, Infinity).text ?? '')
        summary = `Restore ${op.options.path} to the version of ${k.version.at}`
        break
      }
      case 'upload': {
        const dir = at(op.options.dir)
        if (dir?.kind !== 'dir') return refused({ kind: 'not_found' }).events
        const max = this.limits().upload_max
        for (const f of op.options.files) {
          if (!validName(f.name)) return refused({ kind: 'invalid_name' }).events
          if (f.size > max) return refused({ kind: 'too_large', bytes: f.size }).events
          const there = dir.children.get(f.name)
          if (there && there.kind !== 'file') return refused({ kind: 'exists' }).events
          uploads.set(f.name, { name: f.name, size: f.size, exists: !!there, fingerprint: there ? fingerprintOf(there.data, there.meta) : null })
        }
        const n = op.options.files.length
        summary = `Upload ${String(n)} ${n === 1 ? 'file' : 'files'} to ${op.options.dir === '' ? '/' : op.options.dir}`
        break
      }
    }
    const previewId = this.nextPreviewId()
    this.previews.set(previewId, { stack, op, used: false, expired: false, uploads })
    return [
      {
        type: 'FilePreview',
        data: {
          preview_id: previewId,
          stack,
          mount: fm.mount.id,
          summary,
          diff,
          entries,
          ...(op.kind === 'delete' && entries !== null ? { entries_capped: entriesCapped } : {}),
          ...(op.kind === 'upload' ? { uploads: [...uploads.values()] } : {}),
          running: fm.mount.running,
          expires_at: EXPIRES_AT,
        },
      },
    ]
  }

  /** Applies a confirmed op, re-checking what may have changed since the preview. */
  private apply(p: FilePreviewState, fm: FakeMount): Outcome<{ path: string; fingerprint?: Fingerprint }> {
    const { stack, op } = p
    const id = fm.mount.id
    switch (op.kind) {
      case 'save': {
        const { path, base } = op.options
        const n = this.lookup(stack, id, path)
        if (n?.kind !== 'file') return refused({ kind: 'not_found' })
        const current = fingerprintOf(n.data, n.meta)
        if (current !== base) return refused({ kind: 'changed_on_disk', current })
        this.keep(stack, id, path, n.data, 'save')
        return { ok: true, value: { path, fingerprint: this.write(stack, id, path, encodeSave(op.options.text, op.options.bom, op.options.eol)) } }
      }
      case 'create': {
        const path = join([...(partsOf(op.options.parent) ?? []), op.options.name])
        if (this.lookup(stack, id, path)) return refused({ kind: 'exists' })
        const parent = this.lookup(stack, id, op.options.parent)
        const owner = parent ? { uid: parent.meta.uid, gid: parent.meta.gid } : {}
        return { ok: true, value: { path, ...this.create(stack, id, path, op.options.kind, owner) } }
      }
      case 'rename': {
        const { from, to } = op.options
        if (this.lookup(stack, id, to)) return refused({ kind: 'exists' })
        const [srcParent, srcName] = this.parentFor(stack, id, from)
        const node = srcParent.children.get(srcName)
        if (!node) return refused({ kind: 'not_found' })
        srcParent.children.delete(srcName)
        const [dstParent, dstName] = this.parentFor(stack, id, to)
        dstParent.children.set(dstName, node)
        return { ok: true, value: { path: to, ...(node.kind === 'file' ? { fingerprint: fingerprintOf(node.data, node.meta) } : {}) } }
      }
      case 'delete': {
        const { path, raw_name } = op.options
        const n = this.deleteTarget(fm, path, raw_name)
        if (!n) return refused({ kind: 'not_found' })
        if (n.kind === 'file') this.keep(stack, id, path, n.data, 'delete')
        const [parent] = this.parentFor(stack, id, path)
        for (const [k, v] of parent.children) if (v === n) parent.children.delete(k)
        return { ok: true, value: { path } }
      }
      case 'restore': {
        const { path, version } = op.options
        const k = this.kept.get(this.key(stack, id, path))?.find((v) => v.version.id === version)
        if (!k) return refused({ kind: 'not_found' })
        const n = this.lookup(stack, id, path)
        if (n?.kind === 'file') this.keep(stack, id, path, n.data, 'restore')
        return { ok: true, value: { path, fingerprint: this.write(stack, id, path, k.data) } }
      }
      case 'upload':
        // The POSTs write; `Confirm` only opens the preview for them.
        return { ok: true, value: { path: op.options.dir } }
    }
  }

  private create(stack: string, mount: string, path: string, kind: CreateKind, owner: EntryMeta): { fingerprint?: Fingerprint } {
    if (kind === 'dir') {
      this.mkdir(stack, mount, path, owner)
      return {}
    }
    return { fingerprint: this.write(stack, mount, path, '', { mode: 0o644, ...owner }) }
  }

  /** A `Delete` target: by path, or by its raw name in the path's folder. */
  private deleteTarget(fm: FakeMount, path: string, rawName?: string): FakeNode | undefined {
    const parts = partsOf(path)
    if (!parts) return undefined
    if (rawName === undefined) return this.lookupIn(fm, parts)
    const parent = this.lookupIn(fm, parts.slice(0, -1))
    if (parent?.kind !== 'dir') return undefined
    return [...parent.children.values()].find((c) => c.kind === 'file' && c.nameB64 === rawName)
  }

  /** Keeps `data` as a version of the file it is about to be replaced in (R9). */
  private keep(stack: string, mount: string, path: string, data: Uint8Array, reason: VersionReason): void {
    const key = this.key(stack, mount, path)
    const who = this.me()?.identity
    const version: FileVersion = {
      id: `${sha256Hex(data)}-${String(++this.versionSeq)}`,
      at: rfc3339(EPOCH_SECS + this.clock),
      author: who ? `${who.provider}:${who.subject}` : 'github:1001',
      size: data.length,
      reason,
    }
    this.kept.set(key, [{ version, data }, ...(this.kept.get(key) ?? [])].slice(0, HISTORY_MAX))
  }

  private key(stack: string, mount: string, path: string): string {
    return `${stack}\0${mount}\0${join(partsOf(path) ?? [])}`
  }

  private mountOf(stack: string, id: string): FakeMount {
    const m = this.byStack.get(stack)?.find((x) => x.mount.id === id)
    if (!m) throw new Error(`fake-files: ${stack} has no mount ${id}; addMount first`)
    return m
  }

  /** The node at `path`, links not followed. */
  private lookup(stack: string, mount: string, path: string): FakeNode | undefined {
    const parts = partsOf(path)
    const m = this.byStack.get(stack)?.find((x) => x.mount.id === mount)
    return parts && m ? this.lookupIn(m, parts) : undefined
  }

  private lookupIn(m: FakeMount, parts: string[]): FakeNode | undefined {
    let node: FakeNode | undefined = m.root
    for (const part of parts) node = node?.kind === 'dir' ? node.children.get(part) : undefined
    return node
  }

  /** The folder that holds `path` (made as needed) and the last component's name. */
  private parentFor(stack: string, mount: string, path: string): [DirNode, string] {
    const parts = partsOf(path)
    const name = parts?.at(-1)
    if (!parts || name === undefined) throw new Error(`fake-files: not a path inside a mount: ${JSON.stringify(path)}`)
    let dir = this.mountOf(stack, mount).root
    for (const part of parts.slice(0, -1)) {
      const next = dir.children.get(part)
      if (next && next.kind !== 'dir') throw new Error(`fake-files: ${part} is not a folder`)
      const d = next ?? this.dir({})
      dir.children.set(part, d)
      dir = d
    }
    return [dir, name]
  }

  private dir(meta: EntryMeta): DirNode {
    return { kind: 'dir', meta: this.meta({ mode: 0o755, ...meta }), children: new Map() }
  }

  /** Fresh metadata: a new inode and the next tick of the fake clock. */
  private meta(over: Partial<Meta>): Meta {
    this.clock++
    return { uid: 1000, gid: 1000, mode: 0o644, nlink: 1, ...over, ino: ++this.ino, mtime: EPOCH_SECS + this.clock, nanos: 0 }
  }
}
