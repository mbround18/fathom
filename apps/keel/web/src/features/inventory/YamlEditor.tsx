/**
 * A CodeMirror 6 YAML editor for inventories (spec 055 T059), after Depths'
 * `features/editor/CodeEditor`: theme-token highlighting, undo and line wrapping. Loaded lazily, so
 * CodeMirror is its own chunk and only the inventory page pulls it in.
 *
 * The page owns the text: `onChange` reports every edit, and a new `value` from outside (another
 * revision opened) replaces the document. `readOnly` keeps selection and search but refuses edits.
 */
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { yaml } from '@codemirror/lang-yaml'
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { Compartment, EditorState, type Extension } from '@codemirror/state'
import { EditorView, keymap, lineNumbers } from '@codemirror/view'
import { tags as t } from '@lezer/highlight'
import { useEffect, useRef } from 'react'

/** Colours come from the app's tokens, so every theme follows. */
const highlight = HighlightStyle.define([
  { tag: [t.propertyName, t.definition(t.propertyName)], color: 'var(--primary)' },
  { tag: [t.string, t.special(t.string)], color: 'var(--status-ok)' },
  { tag: [t.number, t.bool, t.null, t.atom], color: 'var(--status-warn)' },
  { tag: [t.comment, t.lineComment], color: 'var(--muted-foreground)', fontStyle: 'italic' },
  { tag: [t.keyword, t.meta, t.labelName], color: 'var(--accent-foreground)' },
  { tag: [t.punctuation, t.separator], color: 'var(--muted-foreground)' },
])

const theme = EditorView.theme({
  '&': { fontSize: '12px', backgroundColor: 'transparent', color: 'var(--foreground)' },
  '.cm-content': { fontFamily: 'var(--font-mono, ui-monospace, monospace)', padding: '4px 0' },
  '.cm-gutters': {
    backgroundColor: 'transparent',
    color: 'var(--muted-foreground)',
    borderRight: '1px solid var(--border)',
  },
  '&.cm-focused': { outline: '1px solid var(--ring)' },
})

function readOnlyExtensions(readOnly: boolean): Extension[] {
  return readOnly ? [EditorState.readOnly.of(true), EditorView.contentAttributes.of({ 'aria-readonly': 'true' })] : []
}

export interface YamlEditorProps {
  value: string
  onChange?: (text: string) => void
  label: string
  readOnly?: boolean
}

export default function YamlEditor({ value, onChange, label, readOnly = false }: YamlEditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const changed = useRef(onChange)
  const readOnlyAt = useRef(new Compartment())
  const readOnlyNow = useRef(readOnly)
  useEffect(() => {
    changed.current = onChange
  }, [onChange])

  // One view per editor; `value` and `readOnly` are applied below without recreating it.
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
          theme,
          syntaxHighlighting(highlight),
          yaml(),
          readOnlyAt.current.of(readOnlyExtensions(readOnlyNow.current)),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) changed.current?.(u.state.doc.toString())
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
  }, [label])

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

  return (
    <div
      ref={host}
      data-yaml-editor
      data-read-only={readOnly || undefined}
      className="min-w-0 border border-border bg-background/40 focus-within:border-ring"
    />
  )
}
