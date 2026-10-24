/**
 * A read-only CodeMirror 6 view with YAML highlighting (US1 detail; US3's editor builds on the
 * same setup). Always import it through `React.lazy`: CodeMirror is its own chunk and only loads
 * when a stack's files are on screen (research R10).
 *
 * Lines wrap, so a long image tag or command never makes the page scroll sideways at 360 px.
 */
import { HighlightStyle, StreamLanguage, syntaxHighlighting } from '@codemirror/language'
import { yaml } from '@codemirror/lang-yaml'
import { EditorState, type Extension } from '@codemirror/state'
import { EditorView, lineNumbers } from '@codemirror/view'
import { tags as t } from '@lezer/highlight'
import { useEffect, useRef } from 'react'

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

const baseTheme = EditorView.theme({
  '&': { fontSize: '12px', backgroundColor: 'transparent', color: 'var(--foreground)' },
  '.cm-content': { fontFamily: 'var(--font-mono, ui-monospace, monospace)', padding: '4px 0' },
  '.cm-gutters': {
    backgroundColor: 'transparent',
    color: 'var(--muted-foreground)',
    borderRight: '1px solid var(--border)',
  },
  '&.cm-focused': { outline: '1px solid var(--ring)' },
})

function extensions(language: CodeLanguage, label: string): Extension[] {
  return [
    lineNumbers(),
    EditorView.lineWrapping,
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
    EditorView.contentAttributes.of({ 'aria-label': label, 'aria-readonly': 'true', tabindex: '0' }),
    baseTheme,
    syntaxHighlighting(highlight),
    language === 'yaml' ? yaml() : envLanguage,
  ]
}

export default function CodeView({ text, language, label }: { text: string; language: CodeLanguage; label: string }) {
  const host = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const parent = host.current
    if (!parent) return
    const view = new EditorView({ parent, state: EditorState.create({ doc: text, extensions: extensions(language, label) }) })
    return () => view.destroy()
  }, [text, language, label])

  return <div ref={host} data-code-view={language} className="min-w-0 border border-border bg-background/40" />
}
