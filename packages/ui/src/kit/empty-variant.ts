/**
 * Which empty state a page's empty main list shows (spec 063 R3). A filter wins: a list emptied by
 * one says nothing matches and offers to clear it (FR-014). Otherwise the person either gets the
 * guided steps with the action, or is told who can act (FR-012). Loading and errors never get here.
 */
export type EmptyVariant = 'no-match' | 'guided' | 'restricted'

export function emptyVariant({ filtered, canAct }: { filtered: boolean; canAct: boolean }): EmptyVariant {
  if (filtered) return 'no-match'
  return canAct ? 'guided' : 'restricted'
}
