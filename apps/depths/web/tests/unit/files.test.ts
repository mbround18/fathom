import { describe, expect, it } from 'vitest'

import { matchPath } from '@/app/router'
import {
  filesHref,
  filesLimits,
  filesMode,
  joinPath,
  MOUNT_LIST,
  parentPath,
  parseFilesRest,
  pathParts,
  showFilesTab,
} from '@/features/files/model'
import type { Me } from '@/protocol/generated'

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
