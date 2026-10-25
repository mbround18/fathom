/**
 * An editable CodeMirror 6 view shared by `ComposeEditor`, `EnvEditor` and the volume explorer's
 * file editor (spec 053): highlighting, undo, line wrapping, and `@codemirror/lint` diagnostics
 * for the problems the page passes in. The same problems are listed under the editor, so they
 * read without hovering a gutter mark.
 *
 * The page owns the text: `onChange` reports every edit, and a new `value` from outside (say,
 * "use their version" after a conflict) replaces the document.
 *
 * `language` is the compose editor's `yaml` or `env` (built in, so those editors behave exactly as
 * before), or a file language from `features/files/lang`, whose support loads in its own chunk
 * after the view opens. `readOnly` keeps the cursor, selection and search but refuses edits.
 */
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { lintGutter, setDiagnostics, type Diagnostic } from '@codemirror/lint'
import { Compartment, EditorState, type Extension } from '@codemirror/state'
import { EditorView, keymap, lineNumbers } from '@codemirror/view'
import { useEffect, useRef } from 'react'

import { loadLanguage, type FileLanguage } from '@/features/files/lang'

import { baseTheme, highlighting, languageExtensions, type CodeLanguage } from './cm-setup'
import { PROBLEM_SOURCE_LABEL, problemText, type Problem } from './problems'

export type EditorLanguage = CodeLanguage | FileLanguage

export interface CodeEditorProps {
  value: string
  onChange?: (text: string) => void
  language: EditorLanguage
  label: string
  problems?: readonly Problem[]
  /** Refuse edits; off by default. */
  readOnly?: boolean
}

const NO_PROBLEMS: readonly Problem[] = []

const isBuiltIn = (language: EditorLanguage): language is CodeLanguage => language === 'yaml' || language === 'env'

function readOnlyExtensions(readOnly: boolean): Extension[] {
  return readOnly ? [EditorState.readOnly.of(true), EditorView.contentAttributes.of({ 'aria-readonly': 'true' })] : []
}

function diagnostics(state: EditorState, problems: readonly Problem[]): Diagnostic[] {
  const out: Diagnostic[] = []
  for (const p of problems) {
    if (p.line === null) continue
    const line = state.doc.line(Math.min(Math.max(p.line, 1), state.doc.lines))
    out.push({ from: line.from, to: line.to, severity: p.severity ?? 'error', message: p.message, source: p.source })
  }
  return out
}

export function CodeEditor({ value, onChange, language, label, problems = NO_PROBLEMS, readOnly = false }: CodeEditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const changed = useRef(onChange)
  const readOnlyAt = useRef(new Compartment())
  const readOnlyNow = useRef(readOnly)
  useEffect(() => {
    changed.current = onChange
  }, [onChange])

  // One view per editor; `value` and `readOnly` are applied below without recreating it (keeps
  // undo and focus).
  useEffect(() => {
    const parent = host.current
    if (!parent) return
    const lang = new Compartment()
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
          isBuiltIn(language) ? languageExtensions(language) : [highlighting, lang.of([])],
          readOnlyAt.current.of(readOnlyExtensions(readOnlyNow.current)),
          lintGutter(),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) changed.current?.(u.state.doc.toString())
          }),
        ],
      }),
    })
    view.current = v
    let live = true
    if (!isBuiltIn(language)) {
      // Highlighting arrives with the language's chunk; until then the text reads plain.
      loadLanguage(language).then(
        (ext) => {
          if (live) v.dispatch({ effects: lang.reconfigure(ext) })
        },
        () => undefined,
      )
    }
    return () => {
      live = false
      v.destroy()
      view.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- created once; `value` syncs below
  }, [language, label])

  useEffect(() => {
    readOnlyNow.current = readOnly
    view.current?.dispatch({ effects: readOnlyAt.current.reconfigure(readOnlyExtensions(readOnly)) })
  }, [readOnly])

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
    <div data-code-editor={language} data-read-only={readOnly || undefined} className="flex min-w-0 flex-col gap-1">
      <div
        ref={host}
        className="min-w-0 border border-border bg-background/40 focus-within:border-ring"
      />
      {problems.length > 0 && (
        <ul data-problems aria-label={`Problems in ${label}`} className="flex flex-col gap-0.5 text-xs">
          {problems.map((p, i) => (
            <li
              key={`${p.source}-${String(p.line)}-${String(i)}`}
              data-problem-line={p.line ?? ''}
              data-problem-source={p.source}
              data-problem-severity={p.severity ?? 'error'}
              className={p.severity === 'warning' ? 'break-words text-status-warn' : 'break-words text-status-err'}
            >
              {PROBLEM_SOURCE_LABEL[p.source]}:{' '}
              {problemText(p)}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
