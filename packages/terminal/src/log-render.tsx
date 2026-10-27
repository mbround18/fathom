/**
 * A parsed line as React nodes: styled spans for its ANSI runs, `<mark>` for search hits. Only
 * text children and computed inline styles; nothing from the log is ever markup.
 */
import type { ReactNode } from 'react'

import { ansiCss, type AnsiRun } from './ansi'
import { lowerOf, type ParsedLine } from './log-parse'
import { highlightRanges } from './log-query'

const MARK = 'bg-accent-neon/30 text-foreground'

interface Piece {
  start: number
  end: number
  run: AnsiRun | null
}

/** The text split at every run boundary, so each piece has one style (or none). */
function pieces(text: string, runs: readonly AnsiRun[]): Piece[] {
  const out: Piece[] = []
  let at = 0
  for (const run of runs) {
    if (run.start > at) out.push({ start: at, end: run.start, run: null })
    out.push({ start: run.start, end: run.end, run })
    at = run.end
  }
  if (at < text.length) out.push({ start: at, end: text.length, run: null })
  return out
}

export function renderLineText(line: ParsedLine, highlights: readonly string[]): ReactNode {
  const { text, runs } = line
  let ranges: [number, number][] = []
  if (highlights.length > 0) {
    const lower = lowerOf(line)
    // Lower-casing can change a string's length (e.g. 'İ'); offsets would then be wrong.
    if (lower.length === text.length) ranges = highlightRanges(lower, highlights)
  }
  if (runs.length === 0 && ranges.length === 0) return text

  const nodes: ReactNode[] = []
  let r = 0
  let key = 0
  for (const p of pieces(text, runs)) {
    const parts: ReactNode[] = []
    let at = p.start
    while (r < ranges.length && (ranges[r]?.[0] ?? Infinity) < p.end) {
      const [hs, he] = ranges[r] as [number, number]
      const s = Math.max(hs, p.start)
      const e = Math.min(he, p.end)
      if (s > at) parts.push(text.slice(at, s))
      parts.push(
        <mark key={key++} className={MARK}>
          {text.slice(s, e)}
        </mark>,
      )
      at = e
      if (he <= p.end) r++
      else break
    }
    if (at < p.end) parts.push(text.slice(at, p.end))
    if (p.run) {
      const s = p.run.style
      nodes.push(
        <span
          key={key++}
          data-ansi=""
          data-fg={s.fg ?? undefined}
          data-bg={s.bg ?? undefined}
          data-bold={s.bold ? '' : undefined}
          style={ansiCss(s)}
        >
          {parts}
        </span>,
      )
    } else {
      nodes.push(...parts)
    }
  }
  return nodes
}
