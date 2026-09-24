/**
 * 016-command-palette: mounts the command palette and owns its open/close state, opened via a
 * global Cmd/Ctrl+K shortcut (mirrors `Inspector.tsx`'s `useEscapeToClose` pattern: a dedicated
 * hook rather than a new shortcut registry — research.md decision 5).
 */
import { useEffect, useState } from 'react'

import { CommandPalette } from '@/components/CommandPalette'

/** Cmd+K (macOS) / Ctrl+K (Windows/Linux) toggles the palette from anywhere in the app. */
function useToggleShortcut(onToggle: () => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== 'k' || !(e.metaKey || e.ctrlKey) || e.defaultPrevented) return
      e.preventDefault()
      onToggle()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [onToggle])
}

export function CommandPaletteHost() {
  const [open, setOpen] = useState(false)
  useToggleShortcut(() => setOpen((o) => !o))

  return <CommandPalette open={open} onOpenChange={setOpen} />
}
