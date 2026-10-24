import { beforeEach, describe, expect, it, vi } from 'vitest'

import { matchPath } from '@/app/router'
import {
  downloadHref,
  filesHref,
  filesLimits,
  filesMode,
  joinPath,
  modeString,
  MOUNT_LIST,
  parentPath,
  parseFilesRest,
  pathParts,
  showFilesTab,
} from '@/features/files/model'
import type { Cmd, Entry, Event, EventEnvelope, FileContent } from '@/protocol/generated'
import type { Me } from '@/protocol/generated'

const bus = vi.hoisted(() => ({
  sent: [] as Cmd[],
  listeners: [] as ((e: EventEnvelope) => void)[],
  reconnect: [] as (() => void)[],
}))
vi.mock('@/app/connection', () => ({
  sendCmd: (cmd: Cmd) => {
    bus.sent.push(cmd)
    return bus.sent.length
  },
  onServerEvent: (l: (e: EventEnvelope) => void) => {
    bus.listeners.push(l)
    return () => undefined
  },
  onReconnected: (l: () => void) => {
    bus.reconnect.push(l)
    return () => undefined
  },
  connect: () => undefined,
  disconnect: () => undefined,
}))

const { filterEntries, resetFilesStore, useFiles } = await import('@/features/files/store')

function me(features: Partial<Me['features']> = {}, over: Partial<Me> = {}): Me {
  return {
    identity: { provider: 'github', subject: '1', username: 'ada', email: null },
    csrf_token: 'x',
    role: 'user',
    owner: false,
    stacks: ['web'],
    features: { shell: true, edit: true, host_metrics: false, ...features },
    network: { trusted: true, reason: 'first', network: '203.0.113.7/32' },
    needs_answer: false,
    proxy_warning: false,
    ...over,
  }
}

describe('the Files tab', () => {
  it('is hidden when the server says nothing about files, or off', () => {
    expect(filesMode(me())).toBe('off')
    expect(showFilesTab(me())).toBe(false)
    expect(showFilesTab(me({ files: 'off' }))).toBe(false)
    expect(showFilesTab(null)).toBe(false)
  })

  it('shows read-only and full to people who may edit', () => {
    expect(showFilesTab(me({ files: 'full' }))).toBe(true)
    expect(showFilesTab(me({ files: 'read_only' }))).toBe(true)
  })

  it('is hidden while editing is off, as the Edit button is', () => {
    expect(showFilesTab(me({ files: 'full', edit: false }))).toBe(false)
  })

  it('reads the limits when the server sends them', () => {
    expect(filesLimits(me())).toBeNull()
    const limits = { editor_max: 1, download_max: 2, upload_max: 3 }
    expect(filesLimits(me({}, { limits: { files: limits } }))).toEqual(limits)
  })
})

describe('Files URLs', () => {
  it('build the mount list, a mount root and a path', () => {
    expect(filesHref('web')).toBe('/stacks/web/files')
    expect(filesHref('web', { mount: 'ab12', path: '' })).toBe('/stacks/web/files/ab12')
    expect(filesHref('web', { mount: 'ab12', path: 'world/level.dat' })).toBe('/stacks/web/files/ab12/world/level.dat')
  })

  it('encode each part on its own', () => {
    expect(filesHref('my stack', { mount: 'ab12', path: 'a b/100%#?.txt' })).toBe(
      '/stacks/my%20stack/files/ab12/a%20b/100%25%23%3F.txt',
    )
  })

  it('drop empty and . parts when building', () => {
    expect(filesHref('web', { mount: 'ab12', path: 'a//./b/' })).toBe('/stacks/web/files/ab12/a/b')
  })

  it('round-trip through the router', () => {
    const loc = { mount: 'ab12', path: 'config/ünï cødé/server.properties' }
    const params = matchPath('/stacks/:name/files/*', filesHref('web', loc))
    expect(params?.name).toBe('web')
    expect(parseFilesRest(params?.['*'] ?? 'missing')).toEqual(loc)
  })

  it('parse the mount list and a mount root', () => {
    expect(parseFilesRest('')).toEqual(MOUNT_LIST)
    expect(parseFilesRest('ab12')).toEqual({ mount: 'ab12', path: '' })
    expect(parseFilesRest('ab12/')).toEqual({ mount: 'ab12', path: '' })
  })

  it('refuse .., an encoded slash or NUL, and bad escapes', () => {
    expect(parseFilesRest('ab12/../etc')).toBeNull()
    expect(parseFilesRest('ab12/%2E%2E/etc')).toBeNull()
    expect(parseFilesRest('ab12/a%2Fb')).toBeNull()
    expect(parseFilesRest('ab12/a%00b')).toBeNull()
    expect(parseFilesRest('ab12/%E0%A4%A')).toBeNull()
  })

  it('drop . parts when parsing', () => {
    expect(parseFilesRest('ab12/./a/%2E/b')).toEqual({ mount: 'ab12', path: 'a/b' })
  })
})

describe('the router splat', () => {
  it('matches zero or more trailing segments, still encoded', () => {
    expect(matchPath('/stacks/:name/files/*', '/stacks/web/files')).toEqual({ name: 'web', '*': '' })
    expect(matchPath('/stacks/:name/files/*', '/stacks/web/files/m/a%2Fb/c')).toEqual({ name: 'web', '*': 'm/a%2Fb/c' })
  })

  it('still needs the fixed part', () => {
    expect(matchPath('/stacks/:name/files/*', '/stacks/web')).toBeNull()
    expect(matchPath('/stacks/:name/files/*', '/stacks/web/edit')).toBeNull()
  })

  it('leaves patterns without one exact', () => {
    expect(matchPath('/stacks/:name', '/stacks/web/files')).toBeNull()
    expect(matchPath('/stacks/:name', '/stacks/web')).toEqual({ name: 'web' })
  })
})

describe('mount-relative paths', () => {
  it('split like the server, refusing ..', () => {
    expect(pathParts('')).toEqual([])
    expect(pathParts('a//./b')).toEqual(['a', 'b'])
    expect(pathParts('a/../b')).toBeNull()
  })

  it('go up and down', () => {
    expect(parentPath('a/b/c')).toBe('a/b')
    expect(parentPath('a')).toBe('')
    expect(parentPath('')).toBe('')
    expect(joinPath('', 'a')).toBe('a')
    expect(joinPath('a/b', 'c')).toBe('a/b/c')
  })
})

describe('permission bits and the download route', () => {
  it('print like ls', () => {
    expect(modeString(0o750)).toBe('rwxr-x---')
    expect(modeString(0o644)).toBe('rw-r--r--')
    expect(modeString(0o4755)).toBe('rwsr-xr-x')
    expect(modeString(0o2644)).toBe('rw-r-Sr--')
    expect(modeString(0o1777)).toBe('rwxrwxrwt')
  })

  it('names the mount and path in the query', () => {
    expect(downloadHref('my web', 'ab12', 'config/a b.txt')).toBe(
      '/api/v1/stacks/my%20web/files/download?mount=ab12&path=config%2Fa+b.txt',
    )
  })
})

// --- The explorer store (T034) ---------------------------------------------------------------

function emit(event: Event, reply_to?: number): void {
  for (const l of bus.listeners) l(reply_to === undefined ? { event } : { reply_to, event })
}

/** The last command of `type` sent, with its request id (its 1-based index in `sent`). */
function lastSent<T extends Cmd['type']>(type: T): { id: number; cmd: Extract<Cmd, { type: T }> } {
  for (let i = bus.sent.length - 1; i >= 0; i--) {
    const cmd = bus.sent[i]
    if (cmd?.type === type) return { id: i + 1, cmd: cmd as Extract<Cmd, { type: T }> }
  }
  throw new Error(`no ${type} sent`)
}

function ent(name: string, over: Partial<Entry> = {}): Entry {
  return { name, kind: 'file', size: 1, mtime: '2026-10-23T12:00:00Z', uid: 1000, gid: 1000, mode: 0o644, nlink: 1, write_blocked: null, ...over }
}

function content(text: string): FileContent {
  return {
    fingerprint: 'sha256:x:1:1:1',
    size: text.length,
    text,
    binary_reason: null,
    bom: false,
    eol: 'lf',
    save_eol: 'lf',
    owner: { uid: 1000, gid: 1000, mode: 0o644 },
  }
}

/** Answers the last `ListDir` with `entries`, in pages of `size`, in the order `order` gives. */
function answerDir(entries: Entry[], size = entries.length || 1, order?: number[]): void {
  const { id, cmd } = lastSent('ListDir')
  const count = Math.max(1, Math.ceil(entries.length / size))
  for (const page of order ?? Array.from({ length: count }, (_, i) => i)) {
    const data = { req: cmd.data.req, mount: cmd.data.mount, path: cmd.data.path, entries: entries.slice(page * size, (page + 1) * size), page, last: page === count - 1 }
    emit({ type: 'DirPage', data }, id)
  }
}

function dirPage(id: number, req: number, path: string, entries: Entry[]): void {
  emit({ type: 'DirPage', data: { req, mount: 'm1', path, entries, page: 0, last: true } }, id)
}

const names = (): string[] => useFiles.getState().entries.map((e) => e.name)

describe('explorer store', () => {
  beforeEach(() => {
    bus.sent.length = 0
    resetFilesStore()
  })

  it('asks for the mounts once, and lists a mount root', () => {
    useFiles.getState().go('web', { mount: null, path: '' })
    expect(bus.sent.map((c) => c.type)).toEqual(['ListMounts'])
    emit({ type: 'Mounts', data: { stack: 'web', mounts: [] } }, 1)
    expect(useFiles.getState().mounts).toEqual([])

    useFiles.getState().go('web', { mount: 'm1', path: '' })
    expect(bus.sent.map((c) => c.type)).toEqual(['ListMounts', 'ListDir'])
    expect(lastSent('ListDir').cmd.data).toMatchObject({ stack: 'web', mount: 'm1', path: '' })
    expect(useFiles.getState().list).toBe('loading')
    answerDir([ent('a', { kind: 'dir' }), ent('b')])
    expect(useFiles.getState().list).toBe('ready')
    expect(names()).toEqual(['a', 'b'])
  })

  it('appends pages in the server order, whatever order they arrive in', () => {
    useFiles.getState().go('web', { mount: 'm1', path: '' })
    const entries = [ent('d1', { kind: 'dir' }), ent('d2', { kind: 'dir' }), ent('a'), ent('B'), ent('c')]
    answerDir(entries, 2, [0, 2, 1])
    expect(names()).toEqual(['d1', 'd2', 'a', 'B', 'c'])
    expect(useFiles.getState().list).toBe('ready')
  })

  it('is still loading until every page is in', () => {
    useFiles.getState().go('web', { mount: 'm1', path: '' })
    answerDir([ent('a'), ent('b'), ent('c')], 1, [0, 2])
    expect(useFiles.getState().list).toBe('loading')
    expect(names()).toEqual(['a', 'c'])
  })

  it('drops replies to a superseded request', () => {
    useFiles.getState().go('web', { mount: 'm1', path: '' })
    answerDir([ent('x', { kind: 'dir' }), ent('y', { kind: 'dir' })])
    useFiles.getState().go('web', { mount: 'm1', path: 'x' })
    const x = lastSent('ListDir')
    useFiles.getState().go('web', { mount: 'm1', path: '' })
    useFiles.getState().go('web', { mount: 'm1', path: 'y' })
    // `y` was asked once its parent was listed again.
    answerDir([ent('x', { kind: 'dir' }), ent('y', { kind: 'dir' })])
    const y = lastSent('ListDir')
    expect(y.cmd.data.path).toBe('y')
    dirPage(x.id, x.cmd.data.req, 'x', [ent('from-x')])
    expect(useFiles.getState().dir).toBe('y')
    expect(names()).toEqual([])
    dirPage(y.id, y.cmd.data.req, 'y', [ent('from-y')])
    expect(names()).toEqual(['from-y'])
  })

  it('a refusal ends the listing with the reason', () => {
    useFiles.getState().go('web', { mount: 'm1', path: '' })
    emit({ type: 'FileRefused', data: { kind: 'not_browsable', reason: { code: 'risky_bind', message: 'the host root' } } }, lastSent('ListDir').id)
    expect(useFiles.getState().list).toBe('refused')
    expect(useFiles.getState().listRefusal).toMatchObject({ kind: 'not_browsable' })
  })

  it('opens a file the URL names: its parent is listed, then it is read', () => {
    useFiles.getState().go('web', { mount: 'm1', path: 'config/server.properties' })
    expect(lastSent('ListDir').cmd.data.path).toBe('config')
    answerDir([ent('server.properties')])
    const read = lastSent('ReadFile')
    expect(read.cmd.data.path).toBe('config/server.properties')
    expect(useFiles.getState().file).toEqual({ state: 'loading', path: 'config/server.properties' })
    const data = { req: read.cmd.data.req, mount: 'm1', path: 'config/server.properties', file: content('motd=hi\n') }
    emit({ type: 'FileContent', data }, read.id)
    const file = useFiles.getState().file
    expect(file.state).toBe('open')
    if (file.state === 'open') expect(file.content.text).toBe('motd=hi\n')
    expect(useFiles.getState().dir).toBe('config')

    // Back to the folder closes the file, and asks for nothing again.
    const sent = bus.sent.length
    useFiles.getState().go('web', { mount: 'm1', path: 'config' })
    expect(useFiles.getState().file).toEqual({ state: 'closed' })
    expect(bus.sent.length).toBe(sent)
  })

  it('a folder the URL names under a listed parent is listed itself', () => {
    useFiles.getState().go('web', { mount: 'm1', path: '' })
    answerDir([ent('config', { kind: 'dir' })])
    useFiles.getState().go('web', { mount: 'm1', path: 'config' })
    expect(lastSent('ListDir').cmd.data.path).toBe('config')
    expect(bus.sent.filter((c) => c.type === 'ReadFile')).toHaveLength(0)
  })

  it('an inside link is tried as a folder, and read as a file when it is not one', () => {
    useFiles.getState().go('web', { mount: 'm1', path: '' })
    const root = [ent('cfg', { kind: 'link', link: { target: 'config', inside: true } }), ent('l', { kind: 'link', link: { target: 'a.txt', inside: true } })]
    answerDir(root)
    useFiles.getState().go('web', { mount: 'm1', path: 'cfg' })
    const asFolder = lastSent('ListDir')
    expect(asFolder.cmd.data.path).toBe('cfg')
    // The root stays on screen until the link answers.
    expect(names()).toEqual(['cfg', 'l'])
    dirPage(asFolder.id, asFolder.cmd.data.req, 'cfg', [ent('inside')])
    expect(useFiles.getState().dir).toBe('cfg')
    expect(names()).toEqual(['inside'])

    useFiles.getState().go('web', { mount: 'm1', path: '' })
    answerDir(root)
    useFiles.getState().go('web', { mount: 'm1', path: 'l' })
    emit({ type: 'FileRefused', data: { kind: 'not_found' } }, lastSent('ListDir').id)
    expect(lastSent('ReadFile').cmd.data.path).toBe('l')
    expect(useFiles.getState().dir).toBe('')
  })

  it('a refused read keeps the reason with the path', () => {
    useFiles.getState().go('web', { mount: 'm1', path: 'secret.conf' })
    answerDir([ent('secret.conf')])
    emit({ type: 'Denied', data: { reason: 'untrusted_network' } }, lastSent('ReadFile').id)
    expect(useFiles.getState().file).toEqual({ state: 'refused', path: 'secret.conf', refusal: { kind: 'denied', reason: 'untrusted_network' } })
  })

  it('a content for a file no longer asked for is dropped', () => {
    useFiles.getState().go('web', { mount: 'm1', path: 'a.txt' })
    answerDir([ent('a.txt'), ent('b.txt')])
    const a = lastSent('ReadFile')
    useFiles.getState().go('web', { mount: 'm1', path: 'b.txt' })
    const b = lastSent('ReadFile')
    emit({ type: 'FileContent', data: { req: a.cmd.data.req, mount: 'm1', path: 'a.txt', file: content('A') } }, a.id)
    expect(useFiles.getState().file).toEqual({ state: 'loading', path: 'b.txt' })
    emit({ type: 'FileContent', data: { req: b.cmd.data.req, mount: 'm1', path: 'b.txt', file: content('B') } }, b.id)
    expect(useFiles.getState().file).toMatchObject({ state: 'open', path: 'b.txt' })
  })

  it('a new folder starts unfiltered', () => {
    useFiles.getState().go('web', { mount: 'm1', path: '' })
    answerDir([ent('a', { kind: 'dir' })])
    useFiles.getState().setFilter('x')
    expect(useFiles.getState().filter).toBe('x')
    useFiles.getState().go('web', { mount: 'm1', path: 'a' })
    expect(useFiles.getState().filter).toBe('')
  })

  it('a reconnect asks again for what is on screen', () => {
    useFiles.getState().go('web', { mount: 'm1', path: 'a' })
    bus.sent.length = 0
    for (const l of bus.reconnect) l()
    expect(bus.sent.map((c) => c.type)).toEqual(['ListMounts', 'ListDir'])
  })
})

describe('name filter', () => {
  const entries = [ent('Config', { kind: 'dir' }), ent('logs', { kind: 'dir' }), ent('server.properties'), ent('SERVER.log')]

  it('matches anywhere in the name, ignoring case, in listing order', () => {
    expect(filterEntries(entries, 'server').map((e) => e.name)).toEqual(['server.properties', 'SERVER.log'])
    expect(filterEntries(entries, 'CONF').map((e) => e.name)).toEqual(['Config'])
    expect(filterEntries(entries, '  ')).toBe(entries)
    expect(filterEntries(entries, 'nope')).toEqual([])
  })
})
