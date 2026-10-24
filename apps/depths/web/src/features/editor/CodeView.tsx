/**
 * A read-only CodeMirror 6 view with YAML highlighting (US1 detail; US3's editor builds on the
 * same setup). Always import it through `React.lazy`: CodeMirror is its own chunk and only loads
 * when a stack's files are on screen (research R10).
 *
 * Lines wrap, so a long image tag or command never makes the page scroll sideways at 360 px.
 */
import { EditorState, type Extension } from '@codemirror/state'
import { EditorView, lineNumbers } from '@codemirror/view'
import { useEffect, useRef } from 'react'

import { baseTheme, languageExtensions, type CodeLanguage } from './cm-setup'

function extensions(language: CodeLanguage, label: string): Extension[] {
  return [
    lineNumbers(),
    EditorView.lineWrapping,
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
    EditorView.contentAttributes.of({ 'aria-label': label, 'aria-readonly': 'true', tabindex: '0' }),
    baseTheme,
    ...languageExtensions(language),
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
