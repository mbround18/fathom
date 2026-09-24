/**
 * 033 US4: read-only warning that a binding names a user/group absent from the IdP directory.
 * Never offers to delete or edit anything.
 */
import { useMemo } from 'react'

import { useStore } from '@/store'

export function OrphanedSubjectBadge({ binding }: { binding: string }) {
  const orphans = useStore((s) => s.roles.orphans)
  const names = useMemo(
    () => orphans.filter((o) => o.binding === binding).map((o) => o.subject_name),
    [orphans, binding],
  )
  if (names.length === 0) return null
  return (
    <span
      data-testid="orphaned-subject-badge"
      title={`Not found in the IdP directory: ${names.join(', ')}`}
      className="border border-status-warn/50 bg-status-warn/10 px-1.5 py-0.5 text-xs text-status-warn"
    >
      Orphaned subject{names.length > 1 ? 's' : ''}: {names.join(', ')}
    </span>
  )
}
