import { EditorState } from '@codemirror/state'
import { describe, expect, it } from 'vitest'

import { guessLanguage, languageForName, loadLanguage, sniffLanguage, type FileLanguage } from '@/features/files/lang'

describe('guessLanguage by name', () => {
  it.each<[string, FileLanguage]>([
    ['server.properties', 'properties'],
    ['adminlist.txt', 'text'],
    ['server.cfg', 'ini'],
    ['GameUserSettings.CFG', 'ini'],
    ['Game.ini', 'ini'],
    ['start_server.sh', 'shell'],
    ['config.yml', 'yaml'],
    ['docker-compose.yaml', 'yaml'],
    ['ops.json', 'json'],
    ['pack.mcmeta', 'json'],
    ['Cargo.toml', 'toml'],
    ['ServerConfig.xml', 'xml'],
    ['init.lua', 'lua'],
    ['.env', 'properties'],
  ])('%s is %s', (name, language) => {
    expect(guessLanguage(name)).toBe(language)
  })

  it('a name wins over content', () => {
    expect(guessLanguage('adminlist.txt', '#!/bin/sh\necho hi\n')).toBe('text')
  })

  it('says nothing for no extension, a hidden file or an unknown extension', () => {
    expect(languageForName('start')).toBeNull()
    expect(languageForName('.hidden')).toBeNull()
    expect(languageForName('world.dat')).toBeNull()
    expect(languageForName('trailing.')).toBeNull()
  })
})

describe('guessLanguage by content', () => {
  it('a #!/bin/sh file without an extension is shell', () => {
    expect(guessLanguage('start', '#!/bin/sh\nexec ./server\n')).toBe('shell')
  })

  it('reads the interpreter through env', () => {
    expect(sniffLanguage('#!/usr/bin/env bash\n')).toBe('shell')
    expect(sniffLanguage('#!/usr/bin/env lua\n')).toBe('lua')
    expect(sniffLanguage('#!/usr/bin/python3\n')).toBe('text')
  })

  it('JSON only when it parses', () => {
    expect(sniffLanguage('{"a": [1, 2]}\n')).toBe('json')
    expect(sniffLanguage('[1, 2, 3]')).toBe('json')
    expect(sniffLanguage('{ not json')).toBe('text')
  })

  it('XML from its declaration or first tag', () => {
    expect(sniffLanguage('<?xml version="1.0"?>\n<a/>')).toBe('xml')
    expect(sniffLanguage('\n<Settings>\n</Settings>')).toBe('xml')
  })

  it('INI from a section and a key', () => {
    expect(sniffLanguage('; comment\n[Server]\nPort=27015\n')).toBe('ini')
    expect(sniffLanguage('[Server]\n')).toBe('text')
  })

  it('anything else is text', () => {
    expect(sniffLanguage('')).toBe('text')
    expect(sniffLanguage('Notch\njeb_\n')).toBe('text')
  })
})

describe('loadLanguage', () => {
  it.each<FileLanguage>(['ini', 'json', 'yaml', 'toml', 'xml', 'properties', 'shell', 'lua', 'text'])(
    '%s loads an extension a state accepts',
    async (language) => {
      const ext = await loadLanguage(language)
      expect(() => EditorState.create({ doc: 'a = 1\n', extensions: ext })).not.toThrow()
    },
  )
})
