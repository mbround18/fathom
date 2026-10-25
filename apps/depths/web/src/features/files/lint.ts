/**
 * Parse checks for files opened from a volume (spec 053, T044): JSON (native `JSON.parse`), YAML
 * (`yaml`), TOML (`smol-toml`), XML (`DOMParser`) and INI (a small checker for unclosed sections
 * and lines without `=` or `:`). Each finding is a warning anchored on its line; the editor marks
 * it through `@codemirror/lint`, and saving still works after the person confirms.
 *
 * Other languages are not checked: a game's `.properties`, shell or Lua file has no parser here.
 */
import { parse as parseToml, TomlError } from 'smol-toml'

import { yamlProblems, type Problem } from '@/features/editor/problems'

import type { FileLanguage } from './lang'

/** At most this many INI findings, so a file that is not INI at all does not flood the list. */
const INI_MAX = 20

const warning = (source: Problem['source'], line: number | null, message: string): Problem => ({
  line,
  message,
  source,
  severity: 'warning',
})

/** 1-based line of a 0-based character offset in `text`. */
function lineAt(text: string, offset: number): number {
  let line = 1
  for (let i = 0; i < Math.min(offset, text.length); i++) if (text.charCodeAt(i) === 10) line++
  return line
}

function jsonProblems(text: string, name: string): Problem[] {
  // JSON with comments is common in mods' configs; JSON.parse cannot read it, so it is not checked.
  if (/\.(jsonc|json5)$/i.test(name)) return []
  try {
    JSON.parse(text)
    return []
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e)
    // V8: "... at position 10 (line 3 column 1)"; Firefox: "... at line 3 column 1 of the JSON data".
    const byLine = /line (\d+)/.exec(message)
    const byPos = /position (\d+)/.exec(message)
    const line = byLine ? Number(byLine[1]) : byPos ? lineAt(text, Number(byPos[1])) : null
    const clean = message.replace(/\s*\(line \d+ column \d+\)$/, '').replace(/ in JSON at position \d+$/, '')
    return [warning('json', line, clean)]
  }
}

function tomlProblems(text: string): Problem[] {
  try {
    parseToml(text)
    return []
  } catch (e) {
    if (e instanceof TomlError) return [warning('toml', e.line, e.message.split('\n')[0] ?? e.message)]
    return [warning('toml', null, e instanceof Error ? e.message : String(e))]
  }
}

function xmlProblems(text: string): Problem[] {
  const doc = new DOMParser().parseFromString(text, 'application/xml')
  const err = doc.getElementsByTagName('parsererror')[0]
  if (!err) return []
  const raw = err.textContent
  // Chromium: "error on line 3 at column 4: <message>"; Firefox: "... Line Number 3, Column 4:";
  // jsdom: "3:4: <message>".
  const chromium = /error on line (\d+) at column \d+: ([^\n]*)/.exec(raw)
  if (chromium) return [warning('xml', Number(chromium[1]), chromium[2] ?? raw)]
  const short = /^(\d+):\d+: ([^\n]*)/.exec(raw)
  if (short) return [warning('xml', Number(short[1]), short[2] ?? raw)]
  const firefox = /Line Number (\d+)/.exec(raw)
  return [warning('xml', firefox ? Number(firefox[1]) : null, raw.split('\n')[0] ?? raw)]
}

function iniProblems(text: string): Problem[] {
  const rows = text.split(/\r?\n/)
  const looksIni = rows.some((r) => /^\s*\[/.test(r) || /^[^;#\s][^=:]*[=:]/.test(r))
  // A `.cfg` of console commands (`sv_cheats 0`) maps to INI by its name but is not INI.
  if (!looksIni) return []
  const out: Problem[] = []
  let afterKey = false
  rows.forEach((row, i) => {
    if (out.length >= INI_MAX) return
    const line = i + 1
    const t = row.trim()
    if (t === '' || t.startsWith(';') || t.startsWith('#')) return
    if (t.startsWith('[')) {
      afterKey = false
      if (!t.endsWith(']')) out.push(warning('ini', line, 'Section is not closed with ]'))
      return
    }
    if (/^\s/.test(row) && afterKey) return
    if (t.includes('=') || t.includes(':')) {
      afterKey = true
      return
    }
    afterKey = false
    out.push(warning('ini', line, 'Expected key = value'))
  })
  return out
}

/**
 * Warnings for `text` as `language`. `name` is the file name, which tells JSON with comments
 * (`.jsonc`, `.json5`) apart. Empty when the text parses, or when the language is not checked.
 */
export function parseProblems(language: FileLanguage, text: string, name = ''): Problem[] {
  if (text.trim() === '') return []
  switch (language) {
    case 'json':
      return jsonProblems(text, name)
    case 'yaml':
      return yamlProblems(text).map((p) => ({ ...p, severity: 'warning' as const }))
    case 'toml':
      return tomlProblems(text)
    case 'xml':
      return xmlProblems(text)
    case 'ini':
      return iniProblems(text)
    default:
      return []
  }
}
