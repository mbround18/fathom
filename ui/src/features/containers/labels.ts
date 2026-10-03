/**
 * 048-docker-management US1: the words a project group is labelled with.
 *
 * Separate from the list that renders them because these are the decisions worth testing without
 * a DOM: that the no-project bucket is named rather than left blank, that "not adopted" is said to
 * the operator as what it means for them, and that an unreadable definition keeps the parser's own
 * words including the position in them.
 */
import type { OperationRefusal, Project } from '@/protocol/types'

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

/** The sentence that says what the current adoption means for the operator. */
export function adoptionExplanation(project: Project): string {
  if (project.adoption === 'adopted') {
    return 'Adopted for this session. Lifecycle and removal operations on it are available, each behind its own preview.'
  }
  if (project.adoption === 'not_adopted') {
    return 'Read-only. This daemon is shared, so Fathom will not act on a project until you adopt it.'
  }
  return `Write access comes from ${project.adoption.implicit_via_managed_cluster}, a cluster Fathom manages. It is the cluster's grant, so there is nothing here to release.`
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

/*
 * 048 US2: the two previews/outcome strings that are worth asserting without a DOM.
 */

/** Reclaimable size, in the units a person reads. `null` stays unsaid rather than becoming "0 B". */
export function sizeLabel(bytes: number | null): string | null {
  if (bytes === null) return null
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let n = bytes
  let u = 0
  while (n >= 1024 && u < units.length - 1) {
    n /= 1024
    u += 1
  }
  return `${u === 0 ? String(n) : n.toFixed(1)} ${units[u] ?? 'B'}`
}

/** What a refusal says, in the operator's terms rather than the enum's. */
export function refusalLabel(refusal: OperationRefusal): string {
  switch (refusal.kind) {
    case 'not_adopted':
      return refusal.detail
    case 'preview_stale':
      return 'What is running changed since the preview was taken, so the command it showed is no longer the one that would run. Take a new preview.'
    case 'confirmation_mismatch':
      return 'The typed confirmation did not match, so nothing ran.'
    case 'presence_lost':
      return 'The container runtime is no longer reachable, so nothing ran.'
  }
}
