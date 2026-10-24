/**
 * An editable CodeMirror 6 view shared by `ComposeEditor` and `EnvEditor`: highlighting, undo,
 * line wrapping, and `@codemirror/lint` diagnostics for the problems the page passes in. The
 * same problems are listed under the editor, so they read without hovering a gutter mark.
 *
 * The page owns the text: `onChange` reports every edit, and a new `value` from outside (say,
 * "use their version" after a conflict) replaces the document.
 */
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { lintGutter, setDiagnostics, type Diagnostic } from '@codemirror/lint'
import { EditorState } from '@codemirror/state'
import { EditorView, keymap, lineNumbers } from '@codemirror/view'
import { useEffect, useRef } from 'react'

import { baseTheme, languageExtensions, type CodeLanguage } from './cm-setup'
import { problemText, type Problem } from './problems'

export interface CodeEditorProps {
  value: string
  onChange: (text: string) => void
  language: CodeLanguage
  label: string
  problems: readonly Problem[]
}

function diagnostics(state: EditorState, problems: readonly Problem[]): Diagnostic[] {
  const out: Diagnostic[] = []
  for (const p of problems) {
    if (p.line === null) continue
    const line = state.doc.line(Math.min(Math.max(p.line, 1), state.doc.lines))
    out.push({ from: line.from, to: line.to, severity: 'error', message: p.message, source: p.source })
  }
  return out
}

export function CodeEditor({ value, onChange, language, label, problems }: CodeEditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const changed = useRef(onChange)
  useEffect(() => {
    changed.current = onChange
  }, [onChange])

  // One view per editor; `value` is applied below without recreating it (keeps undo and focus).
  useEffect(() => {
    const parent = host.current
    if (!parent) return
    const v = new EditorView({
      parent,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(),
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
          EditorView.lineWrapping,
          EditorView.contentAttributes.of({ 'aria-label': label, 'aria-multiline': 'true' }),
          baseTheme,
          ...languageExtensions(language),
          lintGutter(),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) changed.current(u.state.doc.toString())
          }),
        ],
      }),
    })
    view.current = v
    return () => {
      v.destroy()
      view.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- created once; `value` syncs below
  }, [language, label])

  useEffect(() => {
    const v = view.current
    if (!v) return
    const current = v.state.doc.toString()
    if (current !== value) v.dispatch({ changes: { from: 0, to: current.length, insert: value } })
  }, [value])

  useEffect(() => {
    const v = view.current
    if (!v) return
    v.dispatch(setDiagnostics(v.state, diagnostics(v.state, problems)))
  }, [problems, value])

  return (
    <div data-code-editor={language} className="flex min-w-0 flex-col gap-1">
      <div
        ref={host}
        className="min-w-0 border border-border bg-background/40 focus-within:border-ring"
      />
      {problems.length > 0 && (
        <ul data-problems aria-label={`Problems in ${label}`} className="flex flex-col gap-0.5 text-xs text-status-err">
          {problems.map((p, i) => (
            <li
              key={`${p.source}-${String(p.line)}-${String(i)}`}
              data-problem-line={p.line ?? ''}
              data-problem-source={p.source}
              className="break-words"
            >
              {p.source === 'server' ? 'Compose: ' : 'YAML: '}
              {problemText(p)}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
