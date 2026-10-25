/**
 * Which language a file in a volume is (spec 053, T028): by its name first, then, for a name that
 * says nothing (no extension, or one we do not know), by sniffing the start of its content. Each
 * language's CodeMirror support is its own chunk, loaded only when a file of that kind opens.
 *
 * Game servers' configs are the target: `server.properties`, `*.cfg`, `*.ini`, JSON and YAML
 * configs, TOML, XML, start scripts and Lua mods. Anything else is plain text.
 */
import type { StreamParser } from '@codemirror/language'
import type { Extension } from '@codemirror/state'

export type FileLanguage = 'ini' | 'json' | 'yaml' | 'toml' | 'xml' | 'properties' | 'shell' | 'lua' | 'text'

/** Extensions, lower-cased, without the dot. */
const BY_EXTENSION: Readonly<Record<string, FileLanguage>> = {
  ini: 'ini',
  cfg: 'ini',
  conf: 'ini',
  cnf: 'ini',
  json: 'json',
  jsonc: 'json',
  json5: 'json',
  mcmeta: 'json',
  yml: 'yaml',
  yaml: 'yaml',
  toml: 'toml',
  xml: 'xml',
  xsd: 'xml',
  plist: 'xml',
  properties: 'properties',
  env: 'properties',
  sh: 'shell',
  bash: 'shell',
  zsh: 'shell',
  lua: 'lua',
  txt: 'text',
  log: 'text',
  md: 'text',
  csv: 'text',
}

/** Whole names (lower-cased) that carry no telling extension. */
const BY_NAME: Readonly<Record<string, FileLanguage>> = {
  '.env': 'properties',
  '.bashrc': 'shell',
  '.profile': 'shell',
  '.bash_profile': 'shell',
}

/** How much of the content sniffing looks at. */
const SNIFF_BYTES = 4096

/** The language for a file name alone; `null` when the name does not say. */
export function languageForName(name: string): FileLanguage | null {
  const lower = name.toLowerCase()
  const whole = BY_NAME[lower]
  if (whole) return whole
  const dot = lower.lastIndexOf('.')
  // `.env` is a whole name above; a leading dot alone is a hidden file, not an extension.
  if (dot <= 0 || dot === lower.length - 1) return null
  return BY_EXTENSION[lower.slice(dot + 1)] ?? null
}

/** The language the start of `text` looks like; `text` when nothing stands out. */
export function sniffLanguage(text: string): FileLanguage {
  const head = text.slice(0, SNIFF_BYTES)
  const shebang = /^#!\s*(\S+)(?:\s+(\S+))?/.exec(head)
  if (shebang) {
    // `#!/bin/sh` or `#!/usr/bin/env bash`: the interpreter is the last path part.
    const prog = (shebang[1] ?? '').endsWith('/env') ? (shebang[2] ?? '') : (shebang[1] ?? '')
    const base = prog.slice(prog.lastIndexOf('/') + 1)
    if (/^(?:ba|z|da|k|a)?sh$/.test(base)) return 'shell'
    if (/^lua(?:jit|[\d.]*)$/.test(base)) return 'lua'
    return 'text'
  }
  const start = head.trimStart()
  if (start.startsWith('<?xml') || /^<[A-Za-z!]/.test(start)) return 'xml'
  if ((start.startsWith('{') || start.startsWith('[')) && looksLikeJson(text)) return 'json'
  // A `[section]` line and a `key=value` line.
  if (/^\s*\[[^\]\n]+\]\s*$/m.test(head) && /^\s*[^#;\s[][^=\n]*=/m.test(head)) return 'ini'
  return 'text'
}

function looksLikeJson(text: string): boolean {
  // Only whole documents parse; a large one that does not fit the sniff window still counts if
  // its start is unmistakable (`{"` or `[{`, `["`, `[1`).
  if (text.length <= 64 * 1024) {
    try {
      JSON.parse(text)
      return true
    } catch {
      return false
    }
  }
  return /^\s*(?:\{\s*"|\[\s*[{["\d-])/.test(text)
}

/** The file's language: its name when that says, else its content. */
export function guessLanguage(name: string, text = ''): FileLanguage {
  return languageForName(name) ?? sniffLanguage(text)
}

/** CodeMirror support for each language, each in its own chunk. Plain text has none. */
const LOADERS: Readonly<Record<FileLanguage, () => Promise<Extension>>> = {
  json: () => import('@codemirror/lang-json').then((m) => m.json()),
  xml: () => import('@codemirror/lang-xml').then((m) => m.xml()),
  yaml: () => import('@codemirror/lang-yaml').then((m) => m.yaml()),
  toml: () => legacy(import('@codemirror/legacy-modes/mode/toml').then((m) => m.toml)),
  // The legacy `properties` mode is the INI one too: `[sections]`, `key=value`, `;` and `#`.
  ini: () => legacy(import('@codemirror/legacy-modes/mode/properties').then((m) => m.properties)),
  properties: () => legacy(import('@codemirror/legacy-modes/mode/properties').then((m) => m.properties)),
  shell: () => legacy(import('@codemirror/legacy-modes/mode/shell').then((m) => m.shell)),
  lua: () => legacy(import('@codemirror/legacy-modes/mode/lua').then((m) => m.lua)),
  text: () => Promise.resolve([]),
}

async function legacy(parser: Promise<StreamParser<unknown>>): Promise<Extension> {
  const [{ StreamLanguage }, p] = await Promise.all([import('@codemirror/language'), parser])
  return StreamLanguage.define(p)
}

/** Loads the CodeMirror extension for `language` (without highlighting styles: see cm-setup). */
export function loadLanguage(language: FileLanguage): Promise<Extension> {
  return LOADERS[language]()
}
