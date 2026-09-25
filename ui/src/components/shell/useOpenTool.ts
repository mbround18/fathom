import type { ToolKind } from '@/protocol/types'
import { useStore } from '@/store'

/**
 * Open a tool pane (or the Fleet dashboard) from anywhere: the stack root on phone/tablet, the
 * overlay tools sheet from laptop up (the dock, where present, keeps its own tabs).
 */
export function useOpenTool(): (tool: ToolKind | 'fleet') => void {
  const widthClass = useStore((s) => s.layout.widthClass)
  const openRoot = useStore((s) => s.openRoot)
  const setShell = useStore((s) => s.setShell)
  const setNavOpen = useStore((s) => s.setNavOpen)
  return (tool) => {
    if (tool === 'fleet') setShell({ fleetOpen: true })
    else if (widthClass === 'phone' || widthClass === 'tablet') openRoot('tool', tool)
    else setShell({ toolsSheet: tool })
    setNavOpen(false)
  }
}
