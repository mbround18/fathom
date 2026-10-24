/**
 * Line-anchored problems for the editors (FR-017): the client's own YAML parse errors, checked
 * on every keystroke, plus the server's compose validation errors (`ValidationFailed`) from the
 * last save attempt. A YAML error disables saving; the server has the last word on compose.
 */
import { parseAllDocuments } from 'yaml'

import type { ValidationError } from '@/protocol/generated'

export interface Problem {
  /** 1-based; `null` when the source gives no line. */
  line: number | null
  message: string
  source: 'yaml' | 'server'
}

/** YAML syntax errors in `text`, at most one per line. Empty when it parses. */
export function yamlProblems(text: string): Problem[] {
  const out: Problem[] = []
  const seen = new Set<number | null>()
  for (const doc of parseAllDocuments(text, { prettyErrors: true })) {
    for (const e of doc.errors) {
      const line = e.linePos?.[0].line ?? null
      if (seen.has(line)) continue
      seen.add(line)
      // `prettyErrors` appends a source excerpt after the first line; the editor shows the line.
      out.push({ line, message: e.message.split('\n')[0] ?? e.message, source: 'yaml' })
    }
  }
  return out
}

export function serverProblems(errors: readonly ValidationError[]): Problem[] {
  return errors.map((e) => ({ line: e.line, message: e.message, source: 'server' as const }))
}

export function problemText(p: Problem): string {
  return p.line === null ? p.message : `Line ${String(p.line)}: ${p.message}`
}
