/**
 * Client-side search and sort for resource lists (T061). Search matches what egui's list views
 * matched: a case-insensitive name prefix, the namespace, or `namespace/name`.
 */

export interface Searchable {
  name: string
  namespace?: string | null
}

/**
 * Compile `q` once for a whole list pass: the trim/lowercase of the query is hoisted out of the
 * per-row loop (10k rows x every keystroke and update tick).
 */
export function compileQuery(q: string): (name: string, namespace?: string | null) => boolean {
  const query = q.trim().toLowerCase()
  if (query === '') return () => true
  return (rawName, rawNamespace) => {
    const name = rawName.toLowerCase()
    if (name.startsWith(query)) return true
    const ns = rawNamespace?.toLowerCase()
    if (ns === undefined || ns === '') return false
    if (ns.startsWith(query)) return true
    return `${ns}/${name}`.startsWith(query)
  }
}

/** True when `row` matches the pane's filter text `q` (blank matches everything). */
export function matchesQuery(row: Searchable, q: string): boolean {
  const query = q.trim().toLowerCase()
  if (query === '') return true
  const name = row.name.toLowerCase()
  if (name.startsWith(query)) return true
  const ns = row.namespace?.toLowerCase()
  if (ns === undefined || ns === '') return false
  if (ns.startsWith(query)) return true
  return `${ns}/${name}`.startsWith(query)
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })

export function compareText(a: string, b: string): number {
  return collator.compare(a, b)
}

export function compareNumber(a: number, b: number): number {
  return a - b
}

function parseTs(ts: string | null | undefined): number | undefined {
  if (!ts) return undefined
  const t = Date.parse(ts)
  return Number.isNaN(t) ? undefined : t
}

/** Youngest first (a later creation timestamp is a smaller age); unknown timestamps sort last. */
export function compareAge(a: string | null | undefined, b: string | null | undefined): number {
  const ta = parseTs(a)
  const tb = parseTs(b)
  if (ta === undefined && tb === undefined) return 0
  if (ta === undefined) return 1
  if (tb === undefined) return -1
  return tb - ta
}

export type Tone = 'ok' | 'warn' | 'err' | 'muted'

/** Glyph + tone for a status word. Status is never shown by colour alone (FR-018). */
export interface StatusTone {
  tone: Tone
  glyph: '●' | '▲' | '✖' | '○'
}

const ERR = /fail|error|crash|backoff|evicted|oomkilled|lost|invalid|denied|forbidden/i
const WARN = /pending|unknown|waiting|terminating|containercreating|progressing|degraded|init/i
const OK = /^(running|succeeded|completed|ready|active|bound|available|true|healthy)$/i

export function statusTone(status: string): StatusTone {
  if (ERR.test(status)) return { tone: 'err', glyph: '✖' }
  if (WARN.test(status)) return { tone: 'warn', glyph: '▲' }
  if (OK.test(status)) return { tone: 'ok', glyph: '●' }
  return { tone: 'muted', glyph: '○' }
}

const SEVERITY: Record<Tone, number> = { err: 0, warn: 1, muted: 2, ok: 3 }

/** Worst first: failures, then warnings, then healthy; ties by text. */
export function compareStatus(a: string, b: string): number {
  return SEVERITY[statusTone(a).tone] - SEVERITY[statusTone(b).tone] || compareText(a, b)
}

export type SortDir = 'asc' | 'desc'

/** Stable, non-mutating sort. `desc` reverses the comparator, not the ties. */
export function sortRows<T>(rows: readonly T[], cmp: (a: T, b: T) => number, dir: SortDir): T[] {
  const sign = dir === 'asc' ? 1 : -1
  return rows
    .map((row, i) => ({ row, i }))
    .sort((x, y) => sign * cmp(x.row, y.row) || x.i - y.i)
    .map((x) => x.row)
}
