/**
 * CodeMirror pieces shared by the read-only `CodeView` and the editors: theme-token highlighting,
 * a `.env` language, and the base theme. Only imported from lazily loaded modules.
 */
import { HighlightStyle, StreamLanguage, syntaxHighlighting } from '@codemirror/language'
import { yaml } from '@codemirror/lang-yaml'
import type { Extension } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { tags as t } from '@lezer/highlight'

export type CodeLanguage = 'yaml' | 'env'

/** Theme colours come from the app's tokens, so light, dark and high-contrast all follow. */
const highlight = HighlightStyle.define([
  { tag: [t.propertyName, t.definition(t.propertyName)], color: 'var(--primary)' },
  { tag: [t.string, t.special(t.string)], color: 'var(--status-ok)' },
  { tag: [t.number, t.bool, t.null, t.atom], color: 'var(--status-warn)' },
  { tag: [t.comment, t.lineComment], color: 'var(--muted-foreground)', fontStyle: 'italic' },
  { tag: [t.keyword, t.meta, t.labelName], color: 'var(--accent-foreground)' },
  { tag: [t.punctuation, t.separator], color: 'var(--muted-foreground)' },
])

/** `.env`: `KEY=value` and `# comments`, nothing more. */
const envLanguage = StreamLanguage.define<null>({
  name: 'dotenv',
  startState: () => null,
  token(stream) {
    if (stream.sol()) {
      stream.eatSpace()
      if (stream.peek() === '#') {
        stream.skipToEnd()
        return 'comment'
      }
      if (stream.match(/^(export\s+)?[^=\s#]+(?=\s*=)/)) return 'propertyName'
    }
    if (stream.eat('=')) return 'punctuation'
    stream.skipToEnd()
    return 'string'
  },
})

export const baseTheme = EditorView.theme({
  '&': { fontSize: '12px', backgroundColor: 'transparent', color: 'var(--foreground)' },
  '.cm-content': { fontFamily: 'var(--font-mono, ui-monospace, monospace)', padding: '4px 0' },
  '.cm-gutters': {
    backgroundColor: 'transparent',
    color: 'var(--muted-foreground)',
    borderRight: '1px solid var(--border)',
  },
  '&.cm-focused': { outline: '1px solid var(--ring)' },
})

/** Highlighting plus the language for `language`. */
export function languageExtensions(language: CodeLanguage): Extension[] {
  return [syntaxHighlighting(highlight), language === 'yaml' ? yaml() : envLanguage]
}
