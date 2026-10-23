/**
 * Global keyboard shortcuts (spec 001 T110): `/` focuses the active list's filter, `g p|d|n`
 * navigates, `?` opens an overlay listing every binding from the `SHORTCUTS` registry.
 */
import { useEffect, useState } from 'react'

import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@companion/ui'
import { GO_TO, isTypingTarget, SHORTCUTS } from '@/lib/shortcuts'
import { useStore } from '@/store'

const CHORD_MS = 1200

export function ShortcutsHost() {
  const [open, setOpen] = useState(false)

  useEffect(() => {
    let chordAt = 0
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return
      if (isTypingTarget(e.target)) return
      const now = Date.now()
      const inChord = now - chordAt < CHORD_MS
      chordAt = 0
      const target = GO_TO[e.key]
      if (inChord && target) {
        e.preventDefault()
        useStore.getState().navigate(target)
        return
      }
      if (e.key === 'g') {
        chordAt = now
      } else if (e.key === '/') {
        const { layout } = useStore.getState()
        const id = layout.focusedList ?? layout.slotPanes.find((p) => p.startsWith('list'))
        const box =
          document.querySelector(`[data-pane-id="${id ?? ''}"] input[type="search"]`) ??
          document.querySelector('[data-pane-kind="list"] input[type="search"]')
        if (box instanceof HTMLElement) {
          e.preventDefault()
          box.focus()
        }
      } else if (e.key === '?') {
        e.preventDefault()
        setOpen(true)
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [])

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent data-testid="shortcuts-overlay">
        <DialogHeader>
          <DialogTitle>Keyboard shortcuts</DialogTitle>
          <DialogDescription>Bindings available across Fathom.</DialogDescription>
        </DialogHeader>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
          {SHORTCUTS.map((s) => (
            <div key={s.keys} className="contents">
              <dt className="font-mono text-accent-neon">{s.keys}</dt>
              <dd>
                {s.description} <span className="text-muted-foreground">({s.scope})</span>
              </dd>
            </div>
          ))}
        </dl>
      </DialogContent>
    </Dialog>
  )
}
