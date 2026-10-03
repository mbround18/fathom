/**
 * 048-docker-management US1: the words a project group is labelled with.
 *
 * Separate from the list that renders them because these are the decisions worth testing without
 * a DOM: that the no-project bucket is named rather than left blank, that "not adopted" is said to
 * the operator as what it means for them, and that an unreadable definition keeps the parser's own
 * words including the position in them.
 */
import type { Project } from '@/protocol/types'

/** The name to show. The no-project bucket is named as such, never left blank (FR-006). */
export function groupLabel(project: Project): string {
  return project.group.kind === 'project' ? project.group.name : 'No compose project'
}

/** A stable key and testid fragment for a group. */
export function groupKey(project: Project): string {
  return project.group.kind === 'project' ? project.group.name : 'unowned'
}

/**
 * What the adoption badge says.
 *
 * "Read-only" rather than "not adopted": the operator needs to know what it means for them, and
 * the heading already says which project it is about.
 */
export function adoptionLabel(project: Project): string {
  if (project.adoption === 'adopted') return 'Adopted'
  if (project.adoption === 'not_adopted') return 'Read-only'
  return `Adopted with ${project.adoption.implicit_via_managed_cluster}`
}

/** What the definition file line says, or `null` when the project declares none. */
export function definitionLabel(project: Project): string | null {
  const state = project.definition_state
  if (state === 'undeclared') return null
  const path = project.definition_file ?? 'its definition file'
  if (state === 'present') return path
  if (state === 'missing') return `${path} — no longer on disk`
  // The parser's own words, kept whole: the position is the actionable part of them.
  return `${path} — cannot be read: ${state.unreadable}`
}
