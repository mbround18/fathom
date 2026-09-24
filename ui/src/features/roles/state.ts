/**
 * 033 US4: role-assignment preview, orphaned subjects and the IdP directory cache, fed by
 * `RoleAssignmentPreview` / `OrphanedSubjectsUpdated` / `IdpDirectoryUpdated`
 * (`store/handlers/system.ts`). Read-only: nothing here writes to the cluster or the IdP.
 */
import type { IdpDirectorySnapshot, OrphanedSubject, SystemEvent } from '@/protocol/types'

export interface RolePreviewState {
  /** Increments per reply so a dialog can tell a fresh reply from a stale one. */
  seq: number
  yaml: string
  renderedSubjectName: string
  warnings: string[]
}

export interface RolesState {
  directory: Record<string, IdpDirectorySnapshot>
  preview: RolePreviewState | null
  previewSeq: number
  orphans: OrphanedSubject[]
}

export function initialRolesState(): RolesState {
  return { directory: {}, preview: null, previewSeq: 0, orphans: [] }
}

export function reduceRoles(s: RolesState, event: SystemEvent): boolean {
  switch (event.type) {
    case 'IdpDirectoryUpdated':
      s.directory[event.data.issuer_id] = event.data
      return true
    case 'RoleAssignmentPreview':
      s.previewSeq += 1
      s.preview = {
        seq: s.previewSeq,
        yaml: event.data.yaml,
        renderedSubjectName: event.data.rendered_subject_name,
        warnings: event.data.warnings,
      }
      return true
    case 'OrphanedSubjectsUpdated':
      s.orphans = event.data.orphans
      return true
    default:
      return false
  }
}
