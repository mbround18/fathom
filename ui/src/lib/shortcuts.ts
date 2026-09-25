/**
 * Keyboard shortcut registry (spec 001 T110). Row navigation (arrows, Home/End, Enter, Space,
 * Delete) lives in `ResourceTable` and Cmd/Ctrl+K in `CommandPaletteHost`; this registry lists
 * every binding for the `?` overlay and owns the global ones: `/`, `?`, and `g` chords.
 */
import { SECTION } from '@/protocol/commands'
import type { SectionKey } from '@/protocol/types'

export interface Shortcut {
  keys: string
  description: string
  scope: 'Global' | 'List'
}

export const SHORTCUTS: readonly Shortcut[] = [
  { keys: 'Ctrl/Cmd+K', description: 'Open the command palette', scope: 'Global' },
  { keys: '/', description: 'Focus the filter of the active list', scope: 'Global' },
  { keys: '?', description: 'Show this shortcut list', scope: 'Global' },
  { keys: 'g p', description: 'Go to Pods', scope: 'Global' },
  { keys: 'g d', description: 'Go to Deployments', scope: 'Global' },
  { keys: 'g n', description: 'Go to Nodes', scope: 'Global' },
  { keys: 'Esc', description: 'Close the inspector, dialog or overlay', scope: 'Global' },
  { keys: 'ArrowDown / ArrowUp', description: 'Move the row cursor', scope: 'List' },
  { keys: 'Home / End', description: 'First / last row', scope: 'List' },
  { keys: 'Enter', description: 'Open the focused row in the inspector', scope: 'List' },
  { keys: 'Space', description: 'Toggle row selection', scope: 'List' },
  { keys: 'Delete', description: 'Delete the focused or selected rows', scope: 'List' },
]

/** Second key of a `g` chord to the section it opens. */
export const GO_TO: Readonly<Record<string, SectionKey>> = {
  p: SECTION.pods,
  d: SECTION.deployments,
  n: SECTION.nodes,
}

/** True when the key event target is somewhere text is typed (shortcuts must not fire there). */
export function isTypingTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false
  return t.isContentEditable || t.matches('input, textarea, select, [role="combobox"]')
}
