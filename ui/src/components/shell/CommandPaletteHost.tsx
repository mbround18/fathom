/**
 * 016-command-palette: mounts the command palette and owns its open/close state, opened via a
 * global Cmd/Ctrl+K shortcut (mirrors `Inspector.tsx`'s `useEscapeToClose` pattern: a dedicated
 * hook rather than a new shortcut registry — research.md decision 5).
 */
import { useEffect } from 'react'

import { CommandPalette } from '@/components/CommandPalette'
import { useStore } from '@/store'

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
  const open = useStore((s) => s.shell.paletteOpen)
  const setShell = useStore((s) => s.setShell)
  useToggleShortcut(() => setShell({ paletteOpen: !useStore.getState().shell.paletteOpen }))

  return <CommandPalette open={open} onOpenChange={(o) => setShell({ paletteOpen: o })} />
}
