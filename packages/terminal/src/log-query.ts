/**
 * The log search box's query language, kept small:
 *
 *   timeout            free text, case-insensitive substring
 *   "connection reset" a quoted phrase
 *   -healthz           exclude lines containing the term (also -"a phrase")
 *   level:error        the detected level (any spelling: err, warning, …); repeat to allow several
 *   -level:debug       exclude a level
 *   key:value          a JSON or logfmt field; `*` is a wildcard (`path:/api/*`); a line without
 *                      the field matches when its text contains `key:value` literally
 *
 * Terms are ANDed. Matching is a linear scan over parsed, cached lines, so a ring of 50,000 lines
 * filters well inside a frame.
 */
import { fieldsOf, LOG_LEVELS, lowerOf, normalizeLevel, type LineCache, type LogLevel, type ParsedLine } from './log-parse'

export interface TextTerm {
  readonly text: string
  readonly negate: boolean
}

export interface FieldTerm {
  readonly key: string
  readonly value: string
  readonly negate: boolean
  readonly pattern: RegExp | null
  /** `key:value` as typed (lower-cased), for lines that have no such field. */
  readonly literal: string
}

export interface LogQuery {
  readonly terms: readonly TextTerm[]
  readonly fields: readonly FieldTerm[]
  /** Allowed levels (any of); empty means any level. */
  readonly levels: readonly LogLevel[]
  readonly notLevels: readonly LogLevel[]
  /** A `level:` term named no level we know: nothing can match it. */
  readonly impossible: boolean
  /** Lower-cased positive text to highlight in results. */
  readonly highlights: readonly string[]
  readonly empty: boolean
}

const FIELD = /^([\w.@/-]{1,64}):(.+)$/s
const LEVEL_KEYS = new Set(['level', 'lvl', 'severity'])

function tokenize(input: string): { token: string; quoted: boolean; negate: boolean }[] {
  const out: { token: string; quoted: boolean; negate: boolean }[] = []
  let i = 0
  const n = input.length
  while (i < n) {
    while (i < n && /\s/.test(input[i] ?? '')) i++
    if (i >= n) break
    let negate = false
    if (input[i] === '-' && i + 1 < n && !/[\s-]/.test(input[i + 1] ?? '')) {
      negate = true
      i++
    }
    if (input[i] === '"') {
      const close = input.indexOf('"', i + 1)
      const end = close < 0 ? n : close
      out.push({ token: input.slice(i + 1, end), quoted: true, negate })
      i = close < 0 ? n : close + 1
      continue
    }
    let token = ''
    while (i < n && !/\s/.test(input[i] ?? '')) {
      // `key:"a quoted value"` keeps its spaces.
      if (input[i] === '"' && token.endsWith(':')) {
        const close = input.indexOf('"', i + 1)
        const end = close < 0 ? n : close
        token += input.slice(i + 1, end)
        i = close < 0 ? n : close + 1
        continue
      }
      token += input[i]
      i++
    }
    out.push({ token, quoted: false, negate })
  }
  return out
}

function globPattern(value: string): RegExp | null {
  if (!value.includes('*')) return null
  const body = value
    .split('*')
    .map((p) => p.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
    .join('.*')
  return new RegExp(`^${body}$`, 's')
}

export function parseQuery(input: string): LogQuery {
  const terms: TextTerm[] = []
  const fields: FieldTerm[] = []
  const levels: LogLevel[] = []
  const notLevels: LogLevel[] = []
  let impossible = false
  for (const { token, quoted, negate } of tokenize(input.slice(0, 2_000))) {
    if (token === '') continue
    const lower = token.toLowerCase()
    const field = quoted ? null : FIELD.exec(lower)
    if (field?.[1] && field[2] !== undefined) {
      const [, key, value] = field
      if (LEVEL_KEYS.has(key)) {
        const level = normalizeLevel(value)
        if (level) (negate ? notLevels : levels).push(level)
        else if (!negate) impossible = true
        continue
      }
      fields.push({ key, value, negate, pattern: globPattern(value), literal: lower })
      continue
    }
    terms.push({ text: lower, negate })
  }
  const highlights = [
    ...terms.filter((t) => !t.negate).map((t) => t.text),
    ...fields.filter((f) => !f.negate).map((f) => f.value.replace(/\*/g, '')).filter((v) => v.length > 0),
  ]
  return {
    terms,
    fields,
    levels,
    notLevels,
    impossible,
    highlights,
    empty: terms.length === 0 && fields.length === 0 && levels.length === 0 && notLevels.length === 0 && !impossible,
  }
}

function fieldMatches(line: ParsedLine, f: FieldTerm): boolean {
  const fields = fieldsOf(line)
  const v = fields.get(f.key)
  if (v === undefined) return lowerOf(line).includes(f.literal)
  return f.pattern ? f.pattern.test(v) : v === f.value
}

/** Whether a line passes the query's text and field terms (levels are checked separately). */
export function matchesText(line: ParsedLine, q: LogQuery): boolean {
  if (q.terms.length > 0) {
    const lower = lowerOf(line)
    for (const t of q.terms) {
      if (lower.includes(t.text) === t.negate) return false
    }
  }
  for (const f of q.fields) {
    if (fieldMatches(line, f) === f.negate) return false
  }
  return true
}

export function matchesLevel(level: LogLevel | null, q: LogQuery): boolean {
  if (q.impossible) return false
  if (q.levels.length > 0 && (level === null || !q.levels.includes(level))) return false
  if (level !== null && q.notLevels.includes(level)) return false
  return true
}

export function matchesQuery(line: ParsedLine, q: LogQuery): boolean {
  return matchesLevel(line.level, q) && matchesText(line, q)
}

export type LevelCounts = Record<LogLevel, number> & { none: number }

export function emptyCounts(): LevelCounts {
  const c = { none: 0 } as LevelCounts
  for (const l of LOG_LEVELS) c[l] = 0
  return c
}

export interface FilterOptions {
  /** Level facet: when non-empty, only these levels pass. */
  facet?: ReadonlySet<LogLevel>
  /** Another narrowing the caller applies (a time window), by line index. */
  extra?: (index: number) => boolean
  cache: LineCache
}

export interface FilterResult {
  /** Indices into the input of the lines that pass. */
  readonly indices: number[]
  /** Per level, the lines that pass everything except the level facet. */
  readonly counts: LevelCounts
}

/** One linear pass: the indices that pass, and facet counts for the chips. */
export function filterLines(lines: readonly string[], q: LogQuery, opts: FilterOptions): FilterResult {
  const indices: number[] = []
  const counts = emptyCounts()
  const facet = opts.facet && opts.facet.size > 0 ? opts.facet : null
  for (let i = 0; i < lines.length; i++) {
    if (opts.extra && !opts.extra(i)) continue
    const line = opts.cache.get(lines[i] ?? '')
    if (!matchesQuery(line, q)) continue
    counts[line.level ?? 'none']++
    if (facet && (line.level === null || !facet.has(line.level))) continue
    indices.push(i)
  }
  return { indices, counts }
}

/** Highlight ranges `[start, end)` of `needles` in `lower`, merged and sorted. */
export function highlightRanges(lower: string, needles: readonly string[], max = 200): [number, number][] {
  const raw: [number, number][] = []
  for (const needle of needles) {
    if (needle === '') continue
    for (let at = lower.indexOf(needle); at >= 0 && raw.length < max; at = lower.indexOf(needle, at + needle.length)) {
      raw.push([at, at + needle.length])
    }
  }
  if (raw.length <= 1) return raw
  raw.sort((a, b) => a[0] - b[0])
  const out: [number, number][] = [raw[0] as [number, number]]
  for (let i = 1; i < raw.length; i++) {
    const cur = raw[i] as [number, number]
    const last = out[out.length - 1] as [number, number]
    if (cur[0] <= last[1]) last[1] = Math.max(last[1], cur[1])
    else out.push(cur)
  }
  return out
}
