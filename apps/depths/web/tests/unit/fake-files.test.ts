/**
 * The fake backend's volume model (053 T026): paging, fingerprints, the preview/confirm flow,
 * history, scripted refusals and the two routes, driven without a browser.
 */
import { beforeEach, describe, expect, it } from 'vitest'

import type { Event, FileOp, FilesMode, Me } from '@/protocol/generated'

import { DIR_PAGE_SIZE, FakeFiles, FILE_LIMITS, mountId, sha256Hex } from '../support/fake-files'

function me(files: FilesMode = 'full', over: Partial<Me> = {}): Me {
  return {
    identity: { provider: 'github', subject: '1001', username: 'ada', email: null },
    csrf_token: 'x',
    role: 'admin',
    owner: false,
    stacks: [],
    features: { shell: true, edit: true, host_metrics: false, files },
    limits: { files: { ...FILE_LIMITS } },
    network: { trusted: true, reason: 'first', network: '203.0.113.7/32' },
    needs_answer: false,
    proxy_warning: false,
    ...over,
  }
}

const enc = new TextEncoder()
const FINGERPRINT = /^sha256:[0-9a-f]{64}:\d+:\d+:\d+$/

let session: Me
let files: FakeFiles
let mount: string
let seq: number
const may = (): boolean => true

beforeEach(() => {
  session = me()
  seq = 0
  files = new FakeFiles(
    () => session,
    () => `pv-${String(++seq)}`,
  )
  mount = files.addMount('web', { source: 'web_data' })
})

type DataOf<T extends Event['type']> = Extract<Event, { type: T }> extends { data: infer D } ? D : never

/** The data of the one event `events` holds, which must be a `type`. */
function only<T extends Event['type']>(events: Event[], type: T): DataOf<T> {
  expect(events.map((e) => e.type)).toEqual([type])
  const [e] = events
  if (!e || !('data' in e)) throw new Error(`no ${type}`)
  return e.data as DataOf<T>
}

function previewId(op: FileOp): string {
  return only(files.handle({ type: 'PreviewFileOp', data: { stack: 'web', op } }, may), 'FilePreview').preview_id
}

describe('fake-files', () => {
  it('hashes and names mounts as the server does', () => {
    expect(sha256Hex(new Uint8Array(0))).toBe('e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855')
    expect(sha256Hex(enc.encode('abc'))).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')
    expect(sha256Hex(enc.encode('a'.repeat(1000)))).toBe('41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3')
    expect(mount).toMatch(/^[0-9a-f]{16}$/)
    expect(mount).toBe(mountId('volume', 'web_data'))
    expect(mount).not.toBe(mountId('bind', 'web_data'))
    expect(only(files.handle({ type: 'ListMounts', data: { stack: 'web' } }, may), 'Mounts').mounts.map((m) => m.id)).toEqual([mount])
  })

  it('pages a large folder at 5,000, folders first, `last` on the final page', () => {
    files.seedFolder('web', mount, 'big', 2 * DIR_PAGE_SIZE + 1)
    files.mkdir('web', mount, 'big/zz-folder')
    const pages = files.handle({ type: 'ListDir', data: { stack: 'web', mount, path: 'big', req: 7 } }, may)
    const data = pages.map((e) => (e.type === 'DirPage' ? e.data : null))
    expect(data.map((d) => [d?.page, d?.entries.length, d?.last, d?.req])).toEqual([
      [0, 5000, false, 7],
      [1, 5000, false, 7],
      [2, 2, true, 7],
    ])
    expect(data[0]?.entries[0]).toMatchObject({ name: 'zz-folder', kind: 'dir', mode: 0o755, nlink: 1, write_blocked: null })
    expect(data[0]?.entries[1]).toMatchObject({ name: 'file-00001.txt', kind: 'file', uid: 1000, gid: 1000, mode: 0o644 })
  })

  it('lists an empty folder as one last page', () => {
    const page = only(files.handle({ type: 'ListDir', data: { stack: 'web', mount, path: '', req: 1 } }, may), 'DirPage')
    expect(page).toMatchObject({ page: 0, last: true, entries: [] })
  })

  it('reads a file with its fingerprint, endings, BOM and owner', () => {
    const fp = files.write('web', mount, 'cfg/a.ini', '﻿x=1\r\ny=2\r\n', { uid: 0, gid: 0, mode: 0o600 })
    expect(fp).toMatch(FINGERPRINT)
    const { file } = only(files.handle({ type: 'ReadFile', data: { stack: 'web', mount, path: 'cfg/a.ini', req: 2 } }, may), 'FileContent')
    expect(file).toMatchObject({ fingerprint: fp, text: 'x=1\ny=2\n', bom: true, eol: 'crlf', save_eol: 'crlf', binary_reason: null })
    expect(file.owner).toEqual({ uid: 0, gid: 0, mode: 0o600 })
  })

  it('offers binary, oversized and hard-linked files read-only', () => {
    files.write('web', mount, 'blob', new Uint8Array([1, 0, 2]))
    files.write('web', mount, 'linked', 'x', { nlink: 2 })
    session = me('full', { limits: { files: { ...FILE_LIMITS, editor_max: 4 } } })
    files.write('web', mount, 'long.txt', 'hello world')
    const read = (path: string) => only(files.handle({ type: 'ReadFile', data: { stack: 'web', mount, path, req: 1 } }, may), 'FileContent').file
    expect(read('blob').binary_reason).toBe('binary')
    expect(read('long.txt')).toMatchObject({ binary_reason: 'too_large' })
    expect(read('long.txt').text).toBeUndefined()
    expect(read('linked').read_only).toBe('hard_link')
  })

  it('applies a save only on confirm, bumps history, and refuses a stale base', () => {
    const base = files.write('web', mount, 'motd.txt', 'old\n')
    files.setRunning('web', mount, ['app'])
    const id = previewId({ kind: 'save', options: { mount, path: 'motd.txt', text: 'new\n', base, bom: false, eol: 'lf' } })
    expect(files.read('web', mount, 'motd.txt')).toBe('old\n')
    const done = only(files.confirm(id, may), 'FileOpDone')
    expect(done).toMatchObject({ preview_id: id, mount, path: 'motd.txt', running: ['app'] })
    expect(done.fingerprint).toMatch(FINGERPRINT)
    expect(done.fingerprint).not.toBe(base)
    expect(files.read('web', mount, 'motd.txt')).toBe('new\n')
    expect(files.history('web', mount, 'motd.txt').map((v) => [v.reason, v.size, v.author])).toEqual([['save', 4, 'github:1001']])
    expect(only(files.confirm(id, may), 'PreviewVoid').reason).toBe('used')

    // Someone else wrote it since: the old base no longer matches.
    const stale = previewId({ kind: 'save', options: { mount, path: 'motd.txt', text: 'mine\n', base, bom: false, eol: 'lf' } })
    const refusal = only(files.confirm(stale, may), 'FileRefused')
    expect(refusal).toEqual({ kind: 'changed_on_disk', current: done.fingerprint })
  })

  it('restores a kept version, keeping the replaced content too', () => {
    const base = files.write('web', mount, 'a.txt', 'one\n')
    files.confirm(previewId({ kind: 'save', options: { mount, path: 'a.txt', text: 'two\n', base, bom: false, eol: 'lf' } }), may)
    const [v] = files.history('web', mount, 'a.txt')
    if (!v) throw new Error('no version')
    const old = only(files.handle({ type: 'ReadVersion', data: { stack: 'web', mount, path: 'a.txt', version: v.id, req: 3 } }, may), 'FileContent')
    expect(old).toMatchObject({ version: v.id, file: { text: 'one\n' } })
    files.confirm(previewId({ kind: 'restore', options: { mount, path: 'a.txt', version: v.id } }), may)
    expect(files.read('web', mount, 'a.txt')).toBe('one\n')
    expect(files.history('web', mount, 'a.txt').map((x) => x.reason)).toEqual(['restore', 'save'])
  })

  it('creates, renames and deletes, counting a folder delete', () => {
    files.seedFolder('web', mount, 'logs', 3)
    files.confirm(previewId({ kind: 'create', options: { mount, parent: 'logs', name: 'new', kind: 'dir' } }), may)
    expect(files.exists('web', mount, 'logs/new')).toBe(true)
    files.confirm(previewId({ kind: 'rename', options: { mount, from: 'logs/new', to: 'kept' } }), may)
    expect(files.exists('web', mount, 'kept')).toBe(true)
    const preview = only(files.handle({ type: 'PreviewFileOp', data: { stack: 'web', op: { kind: 'delete', options: { mount, path: 'logs' } } } }, may), 'FilePreview')
    expect(preview).toMatchObject({ entries: 3, entries_capped: false })
    files.confirm(preview.preview_id, may)
    expect(files.exists('web', mount, 'logs')).toBe(false)
    expect(only(files.handle({ type: 'PreviewFileOp', data: { stack: 'web', op: { kind: 'create', options: { mount, parent: '', name: 'kept', kind: 'file' } } } }, may), 'FileRefused')).toEqual({ kind: 'exists' })
  })

  it('gates by mode and role before anything else', () => {
    const list = { type: 'ListDir', data: { stack: 'web', mount, path: '', req: 1 } } as const
    session = me('off')
    expect(only(files.handle(list, may), 'Denied').reason).toBe('feature_disabled')
    session = me('read_only')
    expect(only(files.handle(list, may), 'DirPage').last).toBe(true)
    const create: FileOp = { kind: 'create', options: { mount, parent: '', name: 'x', kind: 'file' } }
    expect(only(files.handle({ type: 'PreviewFileOp', data: { stack: 'web', op: create } }, may), 'FileRefused').kind).toBe('feature_read_only')
    session = me('full')
    expect(only(files.handle(list, () => false), 'Denied').reason).toBe('not_permitted')
  })

  it('uses scripted refusals once, in order, on the command they wait for', () => {
    const base = files.write('web', mount, 'a.txt', 'x')
    files.refuseNext('os_error', 'Confirm').refuseNext({ kind: 'helper_unavailable', reason: 'no image' })
    const list = { type: 'ListDir', data: { stack: 'web', mount, path: '', req: 1 } } as const
    expect(only(files.handle(list, may), 'FileRefused')).toEqual({ kind: 'helper_unavailable', reason: 'no image' })
    expect(only(files.handle(list, may), 'DirPage').entries).toHaveLength(1)
    const id = previewId({ kind: 'save', options: { mount, path: 'a.txt', text: 'y', base, bom: false, eol: 'lf' } })
    expect(only(files.confirm(id, may), 'FileRefused')).toEqual({ kind: 'os_error', errno: 'EIO', message: 'Input/output error' })
    expect(files.read('web', mount, 'a.txt')).toBe('x')

    files.refuseNext('changed_on_disk')
    expect(only(files.handle({ type: 'ReadFile', data: { stack: 'web', mount, path: 'a.txt', req: 1 } }, may), 'FileRefused')).toEqual({
      kind: 'changed_on_disk',
      current: base,
    })
  })

  it('accepts an upload only after its preview is confirmed, once per name', () => {
    files.write('web', mount, 'up/old.txt', 'v1')
    const body = enc.encode('hello')
    const preview = only(
      files.handle(
        { type: 'PreviewFileOp', data: { stack: 'web', op: { kind: 'upload', options: { mount, dir: 'up', files: [{ name: 'new.txt', size: 5 }, { name: 'old.txt', size: 5 }] } } } },
        may,
      ),
      'FilePreview',
    )
    expect(preview.uploads?.map((u) => [u.name, u.exists, u.fingerprint === null])).toEqual([
      ['new.txt', false, true],
      ['old.txt', true, false],
    ])
    const q = (name: string) => new URLSearchParams({ preview: preview.preview_id, name })
    expect(files.upload('web', q('new.txt'), body, may)).toMatchObject({ status: 403, json: { kind: 'not_confirmed' } })
    expect(only(files.confirm(preview.preview_id, may), 'FileOpDone').path).toBe('up')
    const ok = files.upload('web', q('new.txt'), body, may)
    expect(ok.status).toBe(201)
    expect((ok.json as { fingerprint: string }).fingerprint).toMatch(FINGERPRINT)
    expect(files.upload('web', q('new.txt'), body, may)).toMatchObject({ status: 403, json: { kind: 'not_confirmed' } })
    expect(files.upload('web', q('other.txt'), body, may)).toMatchObject({ status: 400, json: { kind: 'invalid_name' } })
    expect(files.upload('web', q('old.txt'), body, may).status).toBe(201)
    expect(files.read('web', mount, 'up/old.txt')).toBe('hello')
    expect(files.history('web', mount, 'up/old.txt').map((v) => v.reason)).toEqual(['upload'])
  })

  it('downloads a file as itself and a folder as a tar, within the limit', () => {
    files.write('web', mount, 'saves/world.dat', 'abc')
    const file = files.download('web', new URLSearchParams({ mount, path: 'saves/world.dat' }), may)
    expect(file).toMatchObject({ status: 200, headers: { 'content-disposition': 'attachment; filename="world.dat"', 'cache-control': 'no-store' } })
    expect(new TextDecoder().decode(file.body)).toBe('abc')
    const folder = files.download('web', new URLSearchParams({ mount, path: 'saves' }), may)
    expect(folder).toMatchObject({ status: 200, contentType: 'application/x-tar', headers: { 'content-disposition': 'attachment; filename="saves.tar"' } })
    expect((folder.body?.length ?? 0) % 512).toBe(0)
    expect(files.download('web', new URLSearchParams({ mount, path: 'nope' }), may)).toMatchObject({ status: 404, json: { kind: 'not_found' } })
    session = me('full', { limits: { files: { ...FILE_LIMITS, download_max: 2 } } })
    expect(files.download('web', new URLSearchParams({ mount, path: 'saves' }), may)).toMatchObject({ status: 413, json: { kind: 'too_large', bytes: 3 } })
  })

  it('refuses mounts that cannot be browsed or written', () => {
    const tmp = files.addMount('web', { kind: 'tmpfs', source: 'tmp', browsable: { ok: false, reason: { code: 'tmpfs', message: 'in memory' } } })
    const pending = files.addMount('web', { source: 'web_cache', state: 'not_created' })
    const ro = files.addMount('web', { source: 'web_ro', writable: false })
    const list = (m: string) => files.handle({ type: 'ListDir', data: { stack: 'web', mount: m, path: '', req: 1 } }, may)
    expect(only(list(tmp), 'FileRefused')).toEqual({ kind: 'not_browsable', reason: { code: 'tmpfs', message: 'in memory' } })
    expect(only(list(pending), 'FileRefused').kind).toBe('not_created')
    expect(only(list('0000000000000000'), 'FileRefused').kind).toBe('not_found')
    files.write('web', ro, 'a', 'x')
    expect(only(list(ro), 'DirPage').entries[0]?.write_blocked).toBe('read_only_mount')
    const op: FileOp = { kind: 'create', options: { mount: ro, parent: '', name: 'b', kind: 'file' } }
    expect(only(files.handle({ type: 'PreviewFileOp', data: { stack: 'web', op } }, may), 'FileRefused').kind).toBe('read_only_mount')
    expect(only(files.handle({ type: 'ListDir', data: { stack: 'web', mount, path: '../etc', req: 1 } }, may), 'FileRefused').kind).toBe('outside_mount')
  })
})
