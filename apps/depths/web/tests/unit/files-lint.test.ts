import { describe, expect, it } from 'vitest'

import { parseProblems } from '@/features/files/lint'

const lines = (text: string, language: Parameters<typeof parseProblems>[0], name?: string) =>
  parseProblems(language, text, name).map((p) => p.line)

describe('parseProblems (spec 053, T044)', () => {
  it('is empty for valid files of every checked format', () => {
    expect(parseProblems('json', '{\n  "a": 1\n}\n')).toEqual([])
    expect(parseProblems('yaml', 'a: 1\nb:\n  - x\n')).toEqual([])
    expect(parseProblems('toml', 'a = 1\n[server]\nport = 25565\n')).toEqual([])
    expect(parseProblems('xml', '<a>\n  <b/>\n</a>\n')).toEqual([])
    expect(parseProblems('ini', '; comment\n[server]\nport=25565\nname : x\n\n# other\n')).toEqual([])
  })

  it('marks every warning as a warning from the file check', () => {
    const [p] = parseProblems('json', '{\n"a": 1,\n}')
    expect(p).toMatchObject({ severity: 'warning', source: 'json' })
    expect(p?.message).not.toBe('')
  })

  it('JSON: the line of the first error', () => {
    expect(lines('{\n"a": 1,\n}', 'json')).toEqual([3])
    expect(lines('{\n  "a": 1\n  "b": 2\n}', 'json')).toEqual([3])
  })

  it('JSON: comments are allowed in .jsonc and .json5 files', () => {
    expect(parseProblems('json', '{\n// note\n"a": 1\n}', 'settings.jsonc')).toEqual([])
    expect(parseProblems('json', '{\n// note\n"a": 1\n}', 'settings.json5')).toEqual([])
    expect(lines('{\n// note\n"a": 1\n}', 'json', 'settings.json')).toEqual([2])
  })

  it('YAML: the line of the error', () => {
    expect(lines('a: 1\nb: [1, 2\nc: 3\n', 'yaml').length).toBeGreaterThan(0)
    expect(lines('a: 1\nb: c: d\n', 'yaml')).toEqual([2])
  })

  it('TOML: the line of the error', () => {
    expect(lines('a = 1\nb = \n', 'toml')).toEqual([2])
    expect(lines('[server]\nport = 1\nport = 2\n', 'toml')).toEqual([3])
  })

  it('XML: the line of the error', () => {
    expect(lines('<a>\n<b>\n</a>', 'xml')).toEqual([3])
  })

  it('INI: unclosed sections and lines without = or :', () => {
    expect(lines('[server\nport=1\n', 'ini')).toEqual([1])
    expect(lines('[server]\nport=1\njust words\n', 'ini')).toEqual([3])
    const [p] = parseProblems('ini', '[server\nport=1\n')
    expect(p?.message).toMatch(/section/i)
  })

  it('INI: an indented line after a key continues its value', () => {
    expect(parseProblems('ini', '[a]\nlist = one\n  two\n')).toEqual([])
  })

  it('INI: a cfg of console commands (no sections, no key = value) is not checked', () => {
    expect(parseProblems('ini', 'sv_cheats 0\nhostname "My server"\n', 'server.cfg')).toEqual([])
  })

  it('does not check other languages or empty files', () => {
    expect(parseProblems('shell', 'if then fi {{')).toEqual([])
    expect(parseProblems('properties', 'just words')).toEqual([])
    expect(parseProblems('text', '{')).toEqual([])
    expect(parseProblems('json', '  \n')).toEqual([])
  })
})
