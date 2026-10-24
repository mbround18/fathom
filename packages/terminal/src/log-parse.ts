/**
 * What a log line says about itself: its level, and its fields when it is JSON or logfmt.
 *
 * Everything here reads the plain text (`parseAnsi(raw).text`), so colour codes never decide a
 * level and a hostile line cannot smuggle a field through an escape sequence. A line is parsed
 * once per distinct string and the result kept in a bounded cache: the ring is re-filtered on every
 * keystroke and every arriving chunk, and the second pass must cost a map lookup, not a parse.
 */
import { parseAnsi, type AnsiRun } from './ansi'

export const LOG_LEVELS = ['fatal', 'error', 'warn', 'info', 'debug', 'trace'] as const
export type LogLevel = (typeof LOG_LEVELS)[number]

export interface ParsedLine {
  readonly raw: string
  /** The text an operator reads: controls removed. */
  readonly text: string
  readonly runs: readonly AnsiRun[]
  readonly level: LogLevel | null
  /** The parsed object when the line is (or ends in) a JSON object. */
  readonly json: Readonly<Record<string, unknown>> | null
  /** Offset in `text` where the JSON object starts (what precedes it is a prefix). */
  readonly jsonStart: number
  /** `text.toLowerCase()`, computed on first search. */
  lower?: string
  /** Lower-cased field name → lower-cased value, computed on the first `key:value` query. */
  fields?: ReadonlyMap<string, string>
}

const LEVEL_WORDS: Record<string, LogLevel> = {
  fatal: 'fatal',
  panic: 'fatal',
  dpanic: 'fatal',
  crit: 'fatal',
  critical: 'fatal',
  emerg: 'fatal',
  emergency: 'fatal',
  alert: 'fatal',
  error: 'error',
  err: 'error',
  eror: 'error',
  severe: 'error',
  warn: 'warn',
  warning: 'warn',
  wrn: 'warn',
  info: 'info',
  inf: 'info',
  information: 'info',
  informational: 'info',
  notice: 'info',
  debug: 'debug',
  dbg: 'debug',
  debu: 'debug',
  fine: 'debug',
  trace: 'trace',
  trc: 'trace',
  verbose: 'trace',
  finest: 'trace',
}

/** A level name (any common spelling, any case) or a pino/bunyan number. */
export function normalizeLevel(value: unknown): LogLevel | null {
  if (typeof value === 'number') {
    if (value >= 60) return 'fatal'
    if (value >= 50) return 'error'
    if (value >= 40) return 'warn'
    if (value >= 30) return 'info'
    if (value >= 20) return 'debug'
    if (value >= 10) return 'trace'
    return null
  }
  if (typeof value !== 'string' || value.length > 16) return null
  const word = value.trim().toLowerCase()
  if (word === '') return null
  if (/^\d+$/.test(word)) return normalizeLevel(Number(word))
  return LEVEL_WORDS[word] ?? null
}

/** JSON keys that carry a level, in the order they are tried. */
const JSON_LEVEL_KEYS = ['level', 'lvl', 'severity', 'loglevel', 'log.level', 'levelname', '@l', 'severity_text']
/** Only these may hold a number (pino/bunyan); a numeric `severity` is some other scale. */
const NUMERIC_LEVEL_KEYS = new Set(['level', 'lvl'])

/** Lines longer than this are not parsed as JSON (still searchable as text). */
const JSON_MAX_LENGTH = 64 * 1024
/** How far into a line the JSON object may start (timestamps, `service | ` prefixes). */
const JSON_MAX_PREFIX = 256

function findJson(text: string): { json: Record<string, unknown>; start: number } | null {
  if (text.length > JSON_MAX_LENGTH) return null
  let end = text.length
  while (end > 0 && (text[end - 1] === ' ' || text[end - 1] === '\t')) end--
  if (text[end - 1] !== '}') return null
  const start = text.indexOf('{')
  if (start < 0 || start > JSON_MAX_PREFIX) return null
  try {
    const v: unknown = JSON.parse(text.slice(start, end))
    if (v !== null && typeof v === 'object' && !Array.isArray(v)) {
      return { json: v as Record<string, unknown>, start }
    }
  } catch {
    // Not JSON after all; it is text.
  }
  return null
}

function jsonLevel(json: Record<string, unknown>): LogLevel | null {
  for (const key of JSON_LEVEL_KEYS) {
    let v: unknown = json[key]
    if (v === undefined && key.includes('.')) {
      const [a, b] = key.split('.')
      const inner = a === undefined ? undefined : json[a]
      if (inner && typeof inner === 'object' && b !== undefined) v = (inner as Record<string, unknown>)[b]
    }
    if (v === undefined || (typeof v === 'number' && !NUMERIC_LEVEL_KEYS.has(key))) continue
    const level = normalizeLevel(v)
    if (level) return level
  }
  return null
}

const LOGFMT_LEVEL = /(?:^|\s)(?:level|lvl|severity|loglevel)=("?)([A-Za-z]{1,16})\1(?=\s|$)/i
/** klog / glog: `E1023 12:00:00.000000 …`, possibly after a timestamp or prefix. */
const KLOG = /(?:^|\s)([IWEF])\d{4} \d{2}:\d{2}:\d{2}(?:\.\d+)?\s/
const KLOG_LEVEL: Record<string, LogLevel> = { I: 'info', W: 'warn', E: 'error', F: 'fatal' }
const BRACKETED =
  /[[<(]\s*(fatal|panic|crit|critical|emerg|alert|error|err|severe|warn|warning|info|notice|debug|dbg|trace|verbose)\s*[\]>)]/i
const UPPER = /\b(FATAL|PANIC|CRITICAL|CRIT|EMERG|ALERT|ERROR|ERR|SEVERE|WARN|WARNING|INFO|NOTICE|DEBUG|TRACE)\b/
const COLON = /(?:^|\s)(fatal|panic|error|warning|warn|info|debug|trace):\s/i
/** Text forms are looked for near the start: a level word deep in a message is just a word. */
const TEXT_SCAN = 200

/** The level a plain-text line declares, or `null`. */
export function textLevel(text: string): LogLevel | null {
  const m = LOGFMT_LEVEL.exec(text)
  if (m) {
    const level = normalizeLevel(m[2])
    if (level) return level
  }
  const head = text.length > TEXT_SCAN ? text.slice(0, TEXT_SCAN) : text
  const k = KLOG.exec(head)
  if (k?.[1]) return KLOG_LEVEL[k[1]] ?? null
  for (const re of [BRACKETED, UPPER, COLON]) {
    const w = re.exec(head)
    if (w?.[1]) {
      const level = normalizeLevel(w[1])
      if (level) return level
    }
  }
  return null
}

export function parseLine(raw: string): ParsedLine {
  const { text, runs } = parseAnsi(raw)
  const found = findJson(text)
  const level = (found && jsonLevel(found.json)) ?? textLevel(found ? text.slice(0, found.start) : text)
  return { raw, text, runs, level, json: found?.json ?? null, jsonStart: found?.start ?? -1 }
}

/** `text.toLowerCase()`, cached on the line. */
export function lowerOf(line: ParsedLine): string {
  if (line.lower === undefined) line.lower = line.text.toLowerCase()
  return line.lower
}

const LOGFMT_PAIR = /(?:^|\s)([\w.@/-]{1,64})=("(?:[^"\\]|\\.)*"|[^\s"]*)/g
const MAX_FIELDS = 200
const MAX_DEPTH = 4

function flatten(obj: Record<string, unknown>, prefix: string, depth: number, out: Map<string, string>): void {
  for (const [k, v] of Object.entries(obj)) {
    if (out.size >= MAX_FIELDS) return
    const key = (prefix === '' ? k : `${prefix}.${k}`).toLowerCase()
    if (v !== null && typeof v === 'object' && !Array.isArray(v)) {
      if (depth < MAX_DEPTH) flatten(v as Record<string, unknown>, key, depth + 1, out)
    } else if (typeof v === 'string') {
      out.set(key, v.toLowerCase())
    } else {
      out.set(key, JSON.stringify(v).toLowerCase())
    }
  }
}

/** Field name → value for a JSON or logfmt line (both lower-cased), cached on the line. */
export function fieldsOf(line: ParsedLine): ReadonlyMap<string, string> {
  if (line.fields) return line.fields
  const out = new Map<string, string>()
  if (line.json) flatten(line.json, '', 0, out)
  else if (line.text.includes('=')) {
    LOGFMT_PAIR.lastIndex = 0
    for (let m = LOGFMT_PAIR.exec(line.text); m !== null && out.size < MAX_FIELDS; m = LOGFMT_PAIR.exec(line.text)) {
      const key = (m[1] ?? '').toLowerCase()
      let value = m[2] ?? ''
      if (value.startsWith('"')) value = value.slice(1, -1).replace(/\\(.)/g, '$1')
      if (!out.has(key)) out.set(key, value.toLowerCase())
    }
  }
  line.fields = out
  return out
}

/**
 * Parsed lines by raw string. Bounded: past `capacity` the oldest quarter is dropped, so the cache
 * never outgrows a few rings' worth of lines however long a stream runs.
 */
export class LineCache {
  readonly capacity: number
  #map = new Map<string, ParsedLine>()

  constructor(capacity = 64_000) {
    this.capacity = capacity
  }

  get size(): number {
    return this.#map.size
  }

  get(raw: string): ParsedLine {
    let p = this.#map.get(raw)
    if (p === undefined) {
      p = parseLine(raw)
      if (this.#map.size >= this.capacity) this.#evict()
      this.#map.set(raw, p)
    }
    return p
  }

  #evict(): void {
    let drop = Math.max(1, Math.floor(this.capacity / 4))
    for (const key of this.#map.keys()) {
      this.#map.delete(key)
      if (--drop === 0) break
    }
  }
}

/** The cache shared by every log view that does not bring its own. */
export const sharedLineCache = new LineCache()

/**
 * Pretty JSON for the expanded view. `JSON.stringify` escapes C0 controls but not C1 controls or
 * bidi overrides, which could still reorder or hide what is read; those are escaped here.
 */
export function prettyJson(value: unknown): string {
  // eslint-disable-next-line no-control-regex -- escaping controls is the point
  return JSON.stringify(value, null, 2).replace(/[\u007f-\u009f‪-‮⁦-⁩]/g, (c) =>
    `\\u${c.charCodeAt(0).toString(16).padStart(4, '0')}`,
  )
}
